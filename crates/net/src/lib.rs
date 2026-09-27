//! Transport plumbing for mouseshare: framed message send/recv over TCP,
//! connection setup (listen/connect), and the initial Hello/HelloAck
//! handshake used to exchange peer screen info and check protocol version
//! compatibility.
//!
//! This crate intentionally does *not* implement any heartbeat timer or
//! timeout policy — `Message::Heartbeat` is just another message a caller
//! can `send`/`recv` like any other. Timing policy belongs to the
//! application.

use std::net::SocketAddr;

use mouseshare_protocol::{decode_body, encode_frame, Message, ProtocolError, MAX_FRAME_LEN, PROTOCOL_VERSION};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};

/// Errors that can occur while establishing or using a [`Connection`].
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    /// A low-level IO error (other than a clean EOF on read, which is
    /// reported as [`NetError::ConnectionClosed`] instead).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// The peer closed (or reset) the connection. Surfaced when a read
    /// hits EOF, whether that happens cleanly between frames or partway
    /// through a frame.
    #[error("connection closed by peer")]
    ConnectionClosed,

    /// The bytes on the wire didn't decode into a valid `Message`, or a
    /// message failed to encode.
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    /// The peer's length prefix declared a frame larger than
    /// `mouseshare_protocol::MAX_FRAME_LEN`. We refuse to allocate for it.
    #[error("frame of {len} bytes exceeds MAX_FRAME_LEN ({max})")]
    FrameTooLarge { len: u32, max: u32 },

    /// During the handshake, the peer sent a message type other than the
    /// one expected at that step.
    #[error("handshake error: expected {expected}, got {got:?}")]
    UnexpectedMessage { expected: &'static str, got: Message },

    /// The peer's `PROTOCOL_VERSION` doesn't match ours.
    #[error("protocol version mismatch: local={local}, peer={peer}")]
    VersionMismatch { local: u32, peer: u32 },
}

/// Info about the remote peer, learned during the handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerInfo {
    pub screen_id: String,
    pub width: i32,
    pub height: i32,
}

/// A framed, handshaken (once you call one of the `handshake_as_*` methods)
/// connection to a peer.
///
/// `send`/`recv` operate on whole [`Message`]s, handling the 4-byte
/// length-prefix framing defined by `mouseshare_protocol` internally.
pub struct Connection {
    stream: TcpStream,
}

impl Connection {
    fn new(stream: TcpStream) -> Result<Self, NetError> {
        stream.set_nodelay(true)?;
        Ok(Self { stream })
    }

    /// The remote peer's socket address.
    pub fn peer_addr(&self) -> std::io::Result<SocketAddr> {
        self.stream.peer_addr()
    }

    /// Encodes and writes one message to the peer.
    pub async fn send(&mut self, msg: &Message) -> Result<(), NetError> {
        send_to(&mut self.stream, msg).await
    }

    /// Reads one length-prefixed message from the peer. See the
    /// cancellation-safety note on the free function [`recv_from`], which
    /// this delegates to.
    pub async fn recv(&mut self) -> Result<Message, NetError> {
        recv_from(&mut self.stream).await
    }

    /// Splits into independent read/write halves backed by the same TCP
    /// connection, so one task can own [`ConnReader::recv`] in a plain loop
    /// while another independently owns [`ConnWriter::send`] -- avoids the
    /// cancellation hazard documented on `recv()`/[`recv_from`] entirely,
    /// rather than working around it with `select!`.
    pub fn into_split(self) -> (ConnReader, ConnWriter) {
        let (read_half, write_half) = self.stream.into_split();
        (ConnReader { stream: read_half }, ConnWriter { stream: write_half })
    }

    /// Performs the handshake as the side that initiated the TCP
    /// connection (the dialer).
    ///
    /// Sequence: dialer sends `Hello`, then waits for the listener's
    /// `HelloAck`. Returns the peer's [`PeerInfo`] once the listener's
    /// reported `PROTOCOL_VERSION` is confirmed to match ours.
    ///
    /// Takes `&mut self` (rather than consuming `self`) so the caller
    /// still owns the `Connection` afterward for sending/receiving
    /// further messages.
    pub async fn handshake_as_dialer(
        &mut self,
        screen_id: String,
        width: i32,
        height: i32,
    ) -> Result<PeerInfo, NetError> {
        self.send(&Message::Hello {
            version: PROTOCOL_VERSION,
            screen_id,
            width,
            height,
        })
        .await?;

        match self.recv().await? {
            Message::HelloAck {
                version,
                screen_id,
                width,
                height,
            } => {
                if version != PROTOCOL_VERSION {
                    return Err(NetError::VersionMismatch {
                        local: PROTOCOL_VERSION,
                        peer: version,
                    });
                }
                Ok(PeerInfo {
                    screen_id,
                    width,
                    height,
                })
            }
            other => Err(NetError::UnexpectedMessage {
                expected: "HelloAck",
                got: other,
            }),
        }
    }

    /// Performs the handshake as the side that accepted the TCP
    /// connection (the listener).
    ///
    /// Sequence: listener waits for the dialer's `Hello`, checks its
    /// `PROTOCOL_VERSION`, then replies with its own `HelloAck`. Returns
    /// the peer's [`PeerInfo`].
    pub async fn handshake_as_listener(
        &mut self,
        screen_id: String,
        width: i32,
        height: i32,
    ) -> Result<PeerInfo, NetError> {
        match self.recv().await? {
            Message::Hello {
                version,
                screen_id: peer_screen_id,
                width: peer_width,
                height: peer_height,
            } => {
                if version != PROTOCOL_VERSION {
                    return Err(NetError::VersionMismatch {
                        local: PROTOCOL_VERSION,
                        peer: version,
                    });
                }

                self.send(&Message::HelloAck {
                    version: PROTOCOL_VERSION,
                    screen_id,
                    width,
                    height,
                })
                .await?;

                Ok(PeerInfo {
                    screen_id: peer_screen_id,
                    width: peer_width,
                    height: peer_height,
                })
            }
            other => Err(NetError::UnexpectedMessage {
                expected: "Hello",
                got: other,
            }),
        }
    }
}

/// A bound TCP listener that accepts incoming mouseshare connections.
pub struct Listener {
    inner: TcpListener,
}

impl Listener {
    /// The address actually bound to (useful when binding to port 0 and
    /// letting the OS assign a port).
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    /// Accepts one incoming connection, enabling `TCP_NODELAY` on it.
    ///
    /// Does not perform the handshake — call `handshake_as_listener` on
    /// the returned `Connection`.
    pub async fn accept(&self) -> Result<Connection, NetError> {
        let (stream, _peer_addr) = self.inner.accept().await?;
        Connection::new(stream)
    }
}

/// Binds a listener on `addr`. Pass port `0` to let the OS assign a free
/// port, then read it back with `Listener::local_addr`.
pub async fn listen(addr: SocketAddr) -> Result<Listener, NetError> {
    let inner = TcpListener::bind(addr).await?;
    Ok(Listener { inner })
}

/// Dials `addr`, establishing a TCP connection and enabling
/// `TCP_NODELAY` on it.
///
/// Does not perform the handshake — call `handshake_as_dialer` on the
/// returned `Connection`.
pub async fn connect(addr: SocketAddr) -> Result<Connection, NetError> {
    let stream = TcpStream::connect(addr).await?;
    Connection::new(stream)
}

/// The read half of a [`Connection`] split via [`Connection::into_split`].
/// Meant to be owned by a single dedicated task looping on `recv()` — see
/// the cancellation-safety note on [`recv_from`].
pub struct ConnReader {
    stream: OwnedReadHalf,
}

impl ConnReader {
    pub async fn recv(&mut self) -> Result<Message, NetError> {
        recv_from(&mut self.stream).await
    }
}

/// The write half of a [`Connection`] split via [`Connection::into_split`].
pub struct ConnWriter {
    stream: OwnedWriteHalf,
}

impl ConnWriter {
    pub async fn send(&mut self, msg: &Message) -> Result<(), NetError> {
        send_to(&mut self.stream, msg).await
    }
}

async fn send_to<W: AsyncWrite + Unpin>(stream: &mut W, msg: &Message) -> Result<(), NetError> {
    let framed = encode_frame(msg)?;
    stream.write_all(&framed).await?;
    Ok(())
}

/// Reads one length-prefixed message from `stream`.
///
/// Validates the length prefix against `MAX_FRAME_LEN` before allocating a
/// buffer for the body, so a corrupt or malicious prefix can't force an
/// unbounded allocation.
///
/// Not cancellation-safe: this internally performs two `read_exact` calls
/// (one for the 4-byte length prefix, one for the body), and
/// `AsyncReadExt::read_exact` is documented as not being cancel-safe. If
/// this future is dropped mid-read (e.g. it lost a `tokio::select!` race),
/// any bytes already read for the current frame are discarded but the
/// connection's read position on the wire has still moved forward,
/// desynchronizing subsequent framing. Do not call this as a branch in
/// `select!` that might be cancelled; either give it its own dedicated
/// task/loop (as [`ConnReader`] is meant to be used) or make sure the other
/// branches can't fire once a call is in flight.
async fn recv_from<R: AsyncRead + Unpin>(stream: &mut R) -> Result<Message, NetError> {
    let mut len_buf = [0u8; 4];
    read_exact_or_eof(stream, &mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME_LEN {
        return Err(NetError::FrameTooLarge {
            len,
            max: MAX_FRAME_LEN,
        });
    }

    let mut body = vec![0u8; len as usize];
    read_exact_or_eof(stream, &mut body).await?;
    let msg = decode_body(&body)?;
    Ok(msg)
}

/// Like `AsyncReadExt::read_exact`, but maps a clean or mid-frame EOF to
/// `NetError::ConnectionClosed` instead of a generic IO error.
async fn read_exact_or_eof<R: AsyncRead + Unpin>(stream: &mut R, buf: &mut [u8]) -> Result<(), NetError> {
    match stream.read_exact(buf).await {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Err(NetError::ConnectionClosed),
        Err(e) => Err(NetError::Io(e)),
    }
}
