//! Password-authenticated key exchange and the encrypted record layer.
//!
//! Handshake (both sides know the pairing code, nothing else):
//!
//! 1. dialer → listener: SPAKE2 message A (plain, 33 bytes)
//! 2. listener → dialer: SPAKE2 message B (plain, 33 bytes)
//! 3. both derive the SPAKE2 key `K`, then two ChaCha20-Poly1305 keys with
//!    HKDF-SHA256 (salted with the transcript hash, one per direction)
//! 4. dialer sends its encrypted `Hello`; listener answers with an
//!    encrypted `HelloAck`. A successful decrypt in each direction *is* the
//!    mutual key confirmation -- a wrong code produces unrelated keys, the
//!    first AEAD tag fails, and the connection is dropped without any
//!    further information leaking.
//!
//! Why SPAKE2 and not "TLS + the code as a password": a pairing code is
//! short enough to type. With a plain keyed MAC/AEAD an eavesdropper could
//! test guesses offline; with a PAKE every guess costs one live connection
//! attempt, which the listener additionally rate-limits.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};

use crate::pairing::PairingCode;
use crate::NetError;

const ID_DIALER: &[u8] = b"mouseshare-v2-dialer";
const ID_LISTENER: &[u8] = b"mouseshare-v2-listener";

/// Size of the AEAD tag appended to every record.
pub(crate) const TAG_LEN: usize = 16;

/// One direction of the encrypted channel. The nonce is a strictly
/// increasing counter, which is safe because each direction has its own key
/// and a connection never reuses a key.
pub(crate) struct Cipher {
    aead: ChaCha20Poly1305,
    counter: u64,
}

impl Cipher {
    fn new(key: &[u8; 32]) -> Self {
        Self {
            aead: ChaCha20Poly1305::new(Key::from_slice(key)),
            counter: 0,
        }
    }

    fn next_nonce(&mut self) -> Nonce {
        let mut n = [0u8; 12];
        n[4..].copy_from_slice(&self.counter.to_le_bytes());
        self.counter += 1;
        *Nonce::from_slice(&n)
    }

    pub(crate) fn seal(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let nonce = self.next_nonce();
        self.aead
            .encrypt(&nonce, plaintext)
            .expect("ChaCha20-Poly1305 encryption cannot fail for in-memory buffers")
    }

    pub(crate) fn open(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, NetError> {
        let nonce = self.next_nonce();
        self.aead
            .decrypt(&nonce, ciphertext)
            .map_err(|_| NetError::AuthFailed)
    }
}

/// First half of the key exchange, held between sending our message and
/// receiving the peer's.
pub(crate) struct Pending {
    state: Spake2<Ed25519Group>,
    our_msg: Vec<u8>,
    is_dialer: bool,
}

pub(crate) fn start(code: &PairingCode, is_dialer: bool) -> (Pending, Vec<u8>) {
    let password = Password::new(code.as_bytes());
    let (state, msg) = if is_dialer {
        Spake2::<Ed25519Group>::start_a(
            &password,
            &Identity::new(ID_DIALER),
            &Identity::new(ID_LISTENER),
        )
    } else {
        Spake2::<Ed25519Group>::start_b(
            &password,
            &Identity::new(ID_DIALER),
            &Identity::new(ID_LISTENER),
        )
    };
    (
        Pending {
            state,
            our_msg: msg.clone(),
            is_dialer,
        },
        msg,
    )
}

/// Completes the exchange and returns `(send_cipher, recv_cipher)`.
pub(crate) fn finish(pending: Pending, peer_msg: &[u8]) -> Result<(Cipher, Cipher), NetError> {
    let key = pending
        .state
        .finish(peer_msg)
        .map_err(|_| NetError::AuthFailed)?;

    let (msg_a, msg_b) = if pending.is_dialer {
        (pending.our_msg.as_slice(), peer_msg)
    } else {
        (peer_msg, pending.our_msg.as_slice())
    };
    let mut transcript = Sha256::new();
    transcript.update(msg_a);
    transcript.update(msg_b);
    let salt = transcript.finalize();

    let hk = Hkdf::<Sha256>::new(Some(&salt), &key);
    let mut dialer_to_listener = [0u8; 32];
    let mut listener_to_dialer = [0u8; 32];
    hk.expand(b"mouseshare v2 dialer->listener", &mut dialer_to_listener)
        .expect("32 bytes is a valid HKDF output length");
    hk.expand(b"mouseshare v2 listener->dialer", &mut listener_to_dialer)
        .expect("32 bytes is a valid HKDF output length");

    let (send, recv) = if pending.is_dialer {
        (&dialer_to_listener, &listener_to_dialer)
    } else {
        (&listener_to_dialer, &dialer_to_listener)
    };
    Ok((Cipher::new(send), Cipher::new(recv)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(code_a: &PairingCode, code_b: &PairingCode) -> ((Cipher, Cipher), (Cipher, Cipher)) {
        let (pa, ma) = start(code_a, true);
        let (pb, mb) = start(code_b, false);
        let a = finish(pa, &mb).unwrap();
        let b = finish(pb, &ma).unwrap();
        (a, b)
    }

    #[test]
    fn matching_codes_interoperate_in_both_directions() {
        let code = PairingCode::generate();
        let ((mut a_send, mut a_recv), (mut b_send, mut b_recv)) = pair(&code, &code);
        let ct = a_send.seal(b"hello listener");
        assert_eq!(b_recv.open(&ct).unwrap(), b"hello listener");
        let ct = b_send.seal(b"hello dialer");
        assert_eq!(a_recv.open(&ct).unwrap(), b"hello dialer");
    }

    #[test]
    fn different_codes_fail_authentication() {
        let a = PairingCode::generate();
        let b = PairingCode::generate();
        assert_ne!(a, b);
        let ((mut a_send, _), (_, mut b_recv)) = pair(&a, &b);
        let ct = a_send.seal(b"secret");
        assert!(matches!(b_recv.open(&ct), Err(NetError::AuthFailed)));
    }

    #[test]
    fn tampering_and_replay_are_rejected() {
        let code = PairingCode::generate();
        let ((mut a_send, _), (_, mut b_recv)) = pair(&code, &code);
        let mut ct = a_send.seal(b"payload");
        ct[0] ^= 1;
        assert!(b_recv.open(&ct).is_err());

        // A fresh pair: replaying the same ciphertext twice must fail the
        // second time (counter nonce has advanced).
        let ((mut a_send, _), (_, mut b_recv)) = pair(&code, &code);
        let ct = a_send.seal(b"payload");
        assert!(b_recv.open(&ct).is_ok());
        assert!(b_recv.open(&ct).is_err());
    }
}
