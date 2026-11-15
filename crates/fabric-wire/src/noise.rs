//! Noise IK sessions (D17): `Noise_IK_25519_ChaChaPoly_BLAKE2s` through the relay. The caller
//! knows the callee's static key in advance. The callee learns the caller's static key from the
//! first message and can check it against the configured peer before it answers.
//!
//! Transport messages carry an explicit nonce (Noise's stateless transport mode). A receiver
//! accepts only nonces above the last one it accepted, so a relay that replays or reorders
//! frames gets them refused, and a frame that never arrived does not break the session.

use snow::{Builder, HandshakeState, StatelessTransportState};

use crate::error::WireError;
use crate::frame::{Frame, SessionId};

pub const PATTERN: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
/// Both sides mix this into the handshake, so a session of another protocol cannot be reused.
const PROLOGUE: &[u8] = b"fabric-demo-noise-v1";
const MAX_NOISE_BYTES: usize = 65_535;

fn builder<'a>() -> Result<Builder<'a>, WireError> {
    let params = PATTERN
        .parse()
        .map_err(|error: snow::Error| WireError::Noise(error.to_string()))?;
    Ok(Builder::new(params).prologue(PROLOGUE)?)
}

fn write(state: &mut HandshakeState) -> Result<Vec<u8>, WireError> {
    let mut buffer = vec![0; MAX_NOISE_BYTES];
    let length = state.write_message(&[], &mut buffer)?;
    buffer.truncate(length);
    Ok(buffer)
}

fn read(state: &mut HandshakeState, message: &[u8]) -> Result<(), WireError> {
    let mut payload = vec![0; MAX_NOISE_BYTES];
    let length = state.read_message(message, &mut payload)?;
    if length != 0 {
        return Err(WireError::Noise(
            "handshake payloads must be empty".to_owned(),
        ));
    }
    Ok(())
}

/// A call this side placed: it sent `Hello` and waits for `Welcome`.
pub struct Call {
    state: HandshakeState,
    session: SessionId,
}

impl Call {
    /// Places a call to the holder of `remote_static`. Returns the call and its `Hello` frame.
    pub fn place(
        local_secret: &[u8; 32],
        remote_static: &[u8; 32],
        session: SessionId,
    ) -> Result<(Self, Frame), WireError> {
        let mut state = builder()?
            .local_private_key(local_secret)?
            .remote_public_key(remote_static)?
            .build_initiator()?;
        let noise = write(&mut state)?;
        Ok((Self { state, session }, Frame::Hello { session, noise }))
    }

    #[must_use]
    pub const fn session(&self) -> SessionId {
        self.session
    }

    /// Completes the call with the callee's `Welcome` message.
    pub fn complete(mut self, welcome: &[u8]) -> Result<Session, WireError> {
        read(&mut self.state, welcome)?;
        Session::new(self.state, self.session)
    }
}

/// A call this side received: it read `Hello`, knows the caller's static key, and has not
/// answered yet.
pub struct IncomingCall {
    state: HandshakeState,
    session: SessionId,
    caller: [u8; 32],
}

impl IncomingCall {
    /// Reads a `Hello` message. The caller is authenticated but not yet checked against the
    /// configured peer; that is [`IncomingCall::caller`]'s job.
    pub fn receive(
        local_secret: &[u8; 32],
        session: SessionId,
        hello: &[u8],
    ) -> Result<Self, WireError> {
        let mut state = builder()?
            .local_private_key(local_secret)?
            .build_responder()?;
        read(&mut state, hello)?;
        let caller = state
            .get_remote_static()
            .and_then(|key| <[u8; 32]>::try_from(key).ok())
            .ok_or_else(|| WireError::Noise("the caller sent no static key".to_owned()))?;
        Ok(Self {
            state,
            session,
            caller,
        })
    }

    /// The caller's static X25519 public key.
    #[must_use]
    pub const fn caller(&self) -> &[u8; 32] {
        &self.caller
    }

    #[must_use]
    pub const fn session(&self) -> SessionId {
        self.session
    }

    /// Answers the call. Returns the session and the `Welcome` frame.
    pub fn answer(mut self) -> Result<(Session, Frame), WireError> {
        let noise = write(&mut self.state)?;
        let session = self.session;
        Ok((
            Session::new(self.state, session)?,
            Frame::Welcome { session, noise },
        ))
    }
}

/// An established session.
pub struct Session {
    transport: StatelessTransportState,
    id: SessionId,
    next_send: u64,
    next_receive: u64,
}

impl Session {
    fn new(state: HandshakeState, id: SessionId) -> Result<Self, WireError> {
        Ok(Self {
            transport: state.into_stateless_transport_mode()?,
            id,
            next_send: 0,
            next_receive: 0,
        })
    }

    #[must_use]
    pub const fn id(&self) -> SessionId {
        self.id
    }

    /// Encrypts `plaintext` into a transport frame.
    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Frame, WireError> {
        let mut noise = vec![0; MAX_NOISE_BYTES];
        let length = self
            .transport
            .write_message(self.next_send, plaintext, &mut noise)?;
        noise.truncate(length);
        let nonce = self.next_send;
        self.next_send += 1;
        Ok(Frame::Message {
            session: self.id,
            nonce,
            noise,
        })
    }

    /// Decrypts a transport message. Nonces must increase, so a replayed or reordered frame is
    /// refused.
    pub fn open(&mut self, nonce: u64, noise: &[u8]) -> Result<Vec<u8>, WireError> {
        if nonce < self.next_receive {
            return Err(WireError::Noise(format!(
                "nonce {nonce} was already used; the next is at least {}",
                self.next_receive
            )));
        }
        let mut plaintext = vec![0; MAX_NOISE_BYTES];
        let length = self.transport.read_message(nonce, noise, &mut plaintext)?;
        plaintext.truncate(length);
        self.next_receive = nonce + 1;
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use fabric_token::{MemoryStore, PersonKeys};

    use super::{Call, IncomingCall};
    use crate::frame::Frame;

    #[test]
    fn a_call_authenticates_both_sides_and_refuses_replays() {
        let caller = PersonKeys::load_or_create(&MemoryStore::new(), "alice").unwrap();
        let callee = PersonKeys::load_or_create(&MemoryStore::new(), "bob").unwrap();

        let (call, hello) = Call::place(
            &caller.noise_secret(),
            &callee.public().noise_x25519,
            [7; 8],
        )
        .unwrap();
        let Frame::Hello { session, noise } = hello else {
            panic!("a hello frame");
        };
        let incoming = IncomingCall::receive(&callee.noise_secret(), session, &noise).unwrap();
        assert_eq!(incoming.caller(), &caller.public().noise_x25519);
        let (mut callee_session, welcome) = incoming.answer().unwrap();
        let Frame::Welcome { noise, .. } = welcome else {
            panic!("a welcome frame");
        };
        let mut caller_session = call.complete(&noise).unwrap();

        let Frame::Message { nonce, noise, .. } = caller_session.seal(b"hello bob").unwrap() else {
            panic!("a message frame");
        };
        assert_eq!(callee_session.open(nonce, &noise).unwrap(), b"hello bob");
        assert!(callee_session.open(nonce, &noise).is_err(), "replay");
        let Frame::Message { nonce, noise, .. } = callee_session.seal(b"hi alice").unwrap() else {
            panic!("a message frame");
        };
        let mut altered = noise.clone();
        altered[0] ^= 1;
        assert!(caller_session.open(nonce, &altered).is_err(), "altered");
        assert!(
            caller_session.open(nonce + 1, &noise).is_err(),
            "wrong nonce"
        );
        assert_eq!(caller_session.open(nonce, &noise).unwrap(), b"hi alice");
    }
}
