//! Transport plumbing for mouseshare: framed message send/recv over TCP,
//! connection setup (listen/connect), and the initial Hello/HelloAck
//! handshake used to exchange peer screen info and check protocol version
//! compatibility.
//!
//! Every connection is authenticated and encrypted: the handshake runs a
//! SPAKE2 key exchange keyed by the shared [`PairingCode`] before any
//! application message is sent (see the `secure` module for the protocol
//! and its rationale).
//!
//! This crate intentionally does *not* implement any heartbeat timer or
//! timeout policy -- `Message::Ping`/`Pong` are just messages a caller can
//! `send`/`recv` like any other. Timing policy belongs to the application.

mod pairing;
mod secure;

pub use pairing::{PairingCode, PairingCodeError};

use std::net::SocketAddr;

use mouseshare_protocol::{
    decode_body, encode_frame, Message, MonitorInfo, ProtocolError, MAX_FRAME_LEN, PROTOCOL_VERSION,
};
use secure::Cipher;
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
    UnexpectedMessage {
        expected: &'static str,
        got: Message,
    },

    /// The peer's `PROTOCOL_VERSION` doesn't match ours.
    #[error("protocol version mismatch: local={local}, peer={peer}")]
    VersionMismatch { local: u32, peer: u32 },

    /// Key confirmation failed: the pairing codes differ (or the other end
    /// is not a mouseshare peer). Deliberately vague -- it must not tell an
    /// attacker which half was wrong.
    #[error("authentication failed (wrong pairing code?)")]
    AuthFailed,
}

/// Info about the remote peer, learned during the handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerInfo {
    pub device_id: String,
    pub monitors: Vec<MonitorInfo>,
}

/// A framed, handshaken (once you call one of the `handshake_as_*` methods)
/// connection to a peer.
///
/// `send`/`recv` operate on whole [`Message`]s, handling the 4-byte
/// length-prefix framing defined by `mouseshare_protocol` internally.
pub struct Connection {
    stream: TcpStream,
    send_cipher: Option<Cipher>,
    recv_cipher: Option<Cipher>,
}

/// Max size of a raw (pre-encryption) key-exchange frame.
const MAX_RAW_HANDSHAKE_LEN: u32 = 256;

impl Connection {
    fn new(stream: TcpStream) -> Result<Self, NetError> {
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            send_cipher: None,
            recv_cipher: None,
        })
    }

    /// The remote peer's socket address.
    pub fn peer_addr(&self) -> std::io::Result<SocketAddr> {
        self.stream.peer_addr()
    }

    /// Encodes, encrypts and writes one message to the peer.
    pub async fn send(&mut self, msg: &Message) -> Result<(), NetError> {
        send_to(&mut self.stream, self.send_cipher.as_mut(), msg).await
    }

    /// Reads, decrypts and decodes one message from the peer. See the
    /// cancellation-safety note on the free function [`recv_from`], which
    /// this delegates to.
    pub async fn recv(&mut self) -> Result<Message, NetError> {
        recv_from(&mut self.stream, self.recv_cipher.as_mut()).await
    }

    /// Splits into independent read/write halves backed by the same TCP
    /// connection, so one task can own [`ConnReader::recv`] in a plain loop
    /// while another independently owns [`ConnWriter::send`] -- avoids the
    /// cancellation hazard documented on `recv()`/[`recv_from`] entirely,
    /// rather than working around it with `select!`.
    pub fn into_split(self) -> (ConnReader, ConnWriter) {
        let (read_half, write_half) = self.stream.into_split();
        (
            ConnReader {
                stream: read_half,
                cipher: self.recv_cipher,
            },
            ConnWriter {
                stream: write_half,
                cipher: self.send_cipher,
            },
        )
    }

    /// Performs the handshake as the side that initiated the TCP
    /// connection (the dialer): key exchange, then an encrypted `Hello`,
    /// then the listener's encrypted `HelloAck`.
    ///
    /// Fails with [`NetError::AuthFailed`] if the pairing codes differ.
    pub async fn handshake_as_dialer(
        &mut self,
        code: &PairingCode,
        local: &PeerInfo,
    ) -> Result<PeerInfo, NetError> {
        let (pending, our_msg) = secure::start(code, true);
        write_raw(&mut self.stream, &our_msg).await?;
        let peer_msg = match read_raw(&mut self.stream).await {
            Ok(m) => m,
            // The listener hung up before answering: that's what a listener
            // that already rate-limits us looks like, too.
            Err(NetError::ConnectionClosed) => return Err(NetError::AuthFailed),
            Err(e) => return Err(e),
        };
        let (send, recv) = secure::finish(pending, &peer_msg)?;
        self.send_cipher = Some(send);
        self.recv_cipher = Some(recv);

        self.send(&Message::Hello {
            version: PROTOCOL_VERSION,
            device_id: local.device_id.clone(),
            monitors: local.monitors.clone(),
        })
        .await?;

        match self.recv().await {
            Ok(Message::HelloAck {
                version,
                device_id,
                monitors,
            }) => {
                if version != PROTOCOL_VERSION {
                    return Err(NetError::VersionMismatch {
                        local: PROTOCOL_VERSION,
                        peer: version,
                    });
                }
                Ok(PeerInfo {
                    device_id,
                    monitors,
                })
            }
            Ok(other) => Err(NetError::UnexpectedMessage {
                expected: "HelloAck",
                got: other,
            }),
            // A listener that fails key confirmation simply closes.
            Err(NetError::ConnectionClosed) => Err(NetError::AuthFailed),
            Err(e) => Err(e),
        }
    }

    /// Performs the handshake as the side that accepted the TCP
    /// connection (the listener).
    pub async fn handshake_as_listener(
        &mut self,
        code: &PairingCode,
        local: &PeerInfo,
    ) -> Result<PeerInfo, NetError> {
        let peer_msg = read_raw(&mut self.stream).await?;
        let (pending, our_msg) = secure::start(code, false);
        write_raw(&mut self.stream, &our_msg).await?;
        let (send, recv) = secure::finish(pending, &peer_msg)?;
        self.send_cipher = Some(send);
        self.recv_cipher = Some(recv);

        match self.recv().await? {
            Message::Hello {
                version,
                device_id,
                monitors,
            } => {
                if version != PROTOCOL_VERSION {
                    return Err(NetError::VersionMismatch {
                        local: PROTOCOL_VERSION,
                        peer: version,
                    });
                }

                self.send(&Message::HelloAck {
                    version: PROTOCOL_VERSION,
                    device_id: local.device_id.clone(),
                    monitors: local.monitors.clone(),
                })
                .await?;

                Ok(PeerInfo {
                    device_id,
                    monitors,
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
    cipher: Option<Cipher>,
}

impl ConnReader {
    pub async fn recv(&mut self) -> Result<Message, NetError> {
        recv_from(&mut self.stream, self.cipher.as_mut()).await
    }
}

/// The write half of a [`Connection`] split via [`Connection::into_split`].
pub struct ConnWriter {
    stream: OwnedWriteHalf,
    cipher: Option<Cipher>,
}

impl ConnWriter {
    pub async fn send(&mut self, msg: &Message) -> Result<(), NetError> {
        send_to(&mut self.stream, self.cipher.as_mut(), msg).await
    }
}

async fn write_raw<W: AsyncWrite + Unpin>(stream: &mut W, body: &[u8]) -> Result<(), NetError> {
    let mut framed = Vec::with_capacity(4 + body.len());
    framed.extend_from_slice(&(body.len() as u32).to_be_bytes());
    framed.extend_from_slice(body);
    stream.write_all(&framed).await?;
    Ok(())
}

async fn read_raw<R: AsyncRead + Unpin>(stream: &mut R) -> Result<Vec<u8>, NetError> {
    let mut len_buf = [0u8; 4];
    read_exact_or_eof(stream, &mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_RAW_HANDSHAKE_LEN {
        return Err(NetError::FrameTooLarge {
            len,
            max: MAX_RAW_HANDSHAKE_LEN,
        });
    }
    let mut body = vec![0u8; len as usize];
    read_exact_or_eof(stream, &mut body).await?;
    Ok(body)
}

async fn send_to<W: AsyncWrite + Unpin>(
    stream: &mut W,
    cipher: Option<&mut Cipher>,
    msg: &Message,
) -> Result<(), NetError> {
    let framed = encode_frame(msg)?;
    match cipher {
        None => stream.write_all(&framed).await?,
        Some(cipher) => {
            // `framed` is len||body; only the body is encrypted, and the
            // outer length prefix is recomputed to cover the tag too.
            let sealed = cipher.seal(&framed[4..]);
            let mut out = Vec::with_capacity(4 + sealed.len());
            out.extend_from_slice(&(sealed.len() as u32).to_be_bytes());
            out.extend_from_slice(&sealed);
            stream.write_all(&out).await?;
        }
    }
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
/// desynchronizing subsequent framing -- and, with encryption, the nonce
/// counter. Do not call this as a branch in `select!` that might be
/// cancelled; either give it its own dedicated task/loop (as
/// [`ConnReader`] is meant to be used) or make sure the other branches
/// can't fire once a call is in flight.
async fn recv_from<R: AsyncRead + Unpin>(
    stream: &mut R,
    cipher: Option<&mut Cipher>,
) -> Result<Message, NetError> {
    let mut len_buf = [0u8; 4];
    read_exact_or_eof(stream, &mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf);
    let max = MAX_FRAME_LEN + secure::TAG_LEN as u32;
    if len > max {
        return Err(NetError::FrameTooLarge { len, max });
    }

    let mut body = vec![0u8; len as usize];
    read_exact_or_eof(stream, &mut body).await?;
    let plain = match cipher {
        Some(cipher) => cipher.open(&body)?,
        None => body,
    };
    Ok(decode_body(&plain)?)
}

/// Like `AsyncReadExt::read_exact`, but maps a clean or mid-frame EOF to
/// `NetError::ConnectionClosed` instead of a generic IO error.
async fn read_exact_or_eof<R: AsyncRead + Unpin>(
    stream: &mut R,
    buf: &mut [u8],
) -> Result<(), NetError> {
    match stream.read_exact(buf).await {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Err(NetError::ConnectionClosed),
        Err(e) => Err(NetError::Io(e)),
    }
}
