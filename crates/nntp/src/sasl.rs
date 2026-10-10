//! SCRAM-SHA256 messages. Crypto operations are supplied by ring.

use crate::Error;
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::{
    digest, hmac, pbkdf2,
    rand::{SecureRandom, SystemRandom},
};
use std::num::NonZeroU32;

pub(crate) const WIRE_LIMIT: usize = 32 * 1024;
const CREDENTIAL_LIMIT: usize = 4096;

pub(crate) fn encode(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        "=".into()
    } else {
        STANDARD.encode(bytes)
    }
}

pub(crate) fn decode(text: &str) -> Result<Vec<u8>, Error> {
    if text.len() > WIRE_LIMIT {
        return Err(Error::Auth);
    }
    if text == "=" {
        return Ok(Vec::new());
    }
    STANDARD.decode(text).map_err(|_| Error::Auth)
}

pub(crate) fn plain(user: &str, password: &str) -> Result<Vec<u8>, Error> {
    if user.is_empty()
        || user.contains('\0')
        || password.contains('\0')
        || user.len() > CREDENTIAL_LIMIT
        || password.len() > CREDENTIAL_LIMIT
    {
        return Err(Error::Auth);
    }
    let user = stringprep::saslprep(user).map_err(|_| Error::Auth)?;
    if user.is_empty() || user.len() > CREDENTIAL_LIMIT {
        return Err(Error::Auth);
    }
    let mut response = Vec::with_capacity(user.len() + password.len() + 2);
    response.push(0); // Empty authorization identity: derive it from authcid.
    response.extend_from_slice(user.as_bytes());
    response.push(0);
    response.extend_from_slice(password.as_bytes());
    Ok(response)
}

pub(crate) struct Scram {
    nonce: String,
    first_bare: String,
    password: String,
    verification: Option<(hmac::Key, String)>,
}

impl Scram {
    pub(crate) fn new(user: &str, password: &str) -> Result<Self, Error> {
        let mut random = [0u8; 24];
        SystemRandom::new()
            .fill(&mut random)
            .map_err(|_| Error::Auth)?;
        Self::with_nonce(user, password, STANDARD.encode(random))
    }

    fn with_nonce(user: &str, password: &str, nonce: String) -> Result<Self, Error> {
        let user = stringprep::saslprep(user).map_err(|_| Error::Auth)?;
        if user.is_empty() || user.len() > CREDENTIAL_LIMIT || password.len() > CREDENTIAL_LIMIT {
            return Err(Error::Auth);
        }
        let user = user.replace('=', "=3D").replace(',', "=2C");
        Ok(Self {
            first_bare: format!("n={user},r={nonce}"),
            nonce,
            password: password.into(),
            verification: None,
        })
    }

    pub(crate) fn first(&self) -> String {
        format!("n,,{}", self.first_bare)
    }

    pub(crate) fn respond(&mut self, challenge: &[u8]) -> Result<String, Error> {
        let first = std::str::from_utf8(challenge).map_err(|_| Error::Auth)?;
        let attributes = attributes(first)?;
        if attributes.len() < 3
            || attributes[0].0 != "r"
            || attributes[1].0 != "s"
            || attributes[2].0 != "i"
            || attributes.iter().any(|(key, _)| *key == "m")
        {
            return Err(Error::Auth);
        }
        let nonce = attributes[0].1;
        if !nonce.starts_with(&self.nonce)
            || nonce.len() <= self.nonce.len()
            || !nonce
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b',')
        {
            return Err(Error::Auth);
        }
        let salt = decode(attributes[1].1)?;
        let iterations = attributes[2].1;
        if salt.is_empty()
            || salt.len() > 1024
            || iterations.starts_with('0')
            || !iterations.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(Error::Auth);
        }
        let iterations: u32 = iterations.parse().map_err(|_| Error::Auth)?;
        // Bound server-chosen work; typical servers use 4096 or 65536 rounds.
        if iterations > 1_000_000 {
            return Err(Error::Auth);
        }
        let rounds = NonZeroU32::new(iterations).ok_or(Error::Auth)?;
        let password = stringprep::saslprep(&self.password)
            .unwrap_or_else(|_| std::borrow::Cow::Borrowed(&self.password));
        let mut salted = [0u8; 32];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            rounds,
            &salt,
            password.as_bytes(),
            &mut salted,
        );
        let salted_key = hmac::Key::new(hmac::HMAC_SHA256, &salted);
        let client_key = hmac::sign(&salted_key, b"Client Key");
        let stored_key = digest::digest(&digest::SHA256, client_key.as_ref());
        let without_proof = format!("c=biws,r={nonce}");
        let message = format!("{},{first},{without_proof}", self.first_bare);
        let signature = hmac::sign(
            &hmac::Key::new(hmac::HMAC_SHA256, stored_key.as_ref()),
            message.as_bytes(),
        );
        let proof: Vec<u8> = client_key
            .as_ref()
            .iter()
            .zip(signature.as_ref())
            .map(|(left, right)| left ^ right)
            .collect();
        let server_key = hmac::sign(&salted_key, b"Server Key");
        self.verification = Some((
            hmac::Key::new(hmac::HMAC_SHA256, server_key.as_ref()),
            message,
        ));
        self.password.clear();
        Ok(format!("{without_proof},p={}", encode(&proof)))
    }

    pub(crate) fn verify(&self, final_message: &[u8]) -> Result<(), Error> {
        let text = std::str::from_utf8(final_message).map_err(|_| Error::Auth)?;
        let attributes = attributes(text)?;
        if attributes.first().map(|(key, _)| *key) != Some("v")
            || attributes.iter().any(|(key, _)| matches!(*key, "e" | "m"))
        {
            return Err(Error::Auth);
        }
        let signature = decode(attributes[0].1)?;
        let (key, message) = self.verification.as_ref().ok_or(Error::Auth)?;
        // ring compares the server proof in constant time.
        hmac::verify(key, message.as_bytes(), &signature).map_err(|_| Error::Auth)
    }
}

fn attributes(message: &str) -> Result<Vec<(&str, &str)>, Error> {
    let mut values = Vec::new();
    for item in message.split(',') {
        let (name, value) = item.split_once('=').ok_or(Error::Auth)?;
        if name.len() != 1
            || !name.as_bytes()[0].is_ascii_alphabetic()
            || value.is_empty()
            || values.iter().any(|(existing, _)| *existing == name)
        {
            return Err(Error::Auth);
        }
        values.push((name, value));
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONCE: &str = "rOprNGfwEbeRWgbNEkqO";
    const FIRST: &str =
        "r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096";

    #[test]
    fn scram_sha256_matches_rfc7677_and_verifies_server() {
        let mut scram = Scram::with_nonce("user", "pencil", NONCE.into()).unwrap();
        assert_eq!(scram.first(), format!("n,,n=user,r={NONCE}"));
        assert_eq!(scram.respond(FIRST.as_bytes()).unwrap(), "c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ=");
        scram
            .verify(b"v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=")
            .unwrap();
        assert!(scram
            .verify(b"v=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
            .is_err());
        assert!(scram.verify(b"e=invalid-proof").is_err());
    }

    #[test]
    fn scram_rejects_nonce_substitution_extensions_and_unbounded_work() {
        for challenge in [
            "r=wrongnonce,s=YWJj,i=4096".into(),
            format!("r={NONCE},s=YWJj,i=4096"),
            format!("r={NONCE}extra,s=YWJj,i=0"),
            format!("r={NONCE}extra,s=YWJj,i=1000001"),
            format!("r={NONCE}extra,s=YWJj,i=4096,m=required"),
            format!("r={NONCE}extra,s=YWJj,i=4096,r=duplicate"),
            format!("s=YWJj,r={NONCE}extra,i=4096"),
        ] {
            let mut scram = Scram::with_nonce("user", "pencil", NONCE.into()).unwrap();
            assert!(scram.respond(challenge.as_bytes()).is_err(), "{challenge}");
        }
    }

    #[test]
    fn sasl_encoding_preparation_and_empty_challenges_are_strict() {
        assert_eq!(encode(b""), "=");
        assert_eq!(decode("=").unwrap(), b"");
        assert!(decode("Y Q==").is_err());
        assert!(decode("YQ").is_err());
        assert_eq!(plain("I\u{00ad}X", "pass").unwrap(), b"\0IX\0pass");
        assert!(plain("\u{00ad}", "pass").is_err());
        assert!(plain("user", "bad\0password").is_err());
        let scram = Scram::with_nonce("a,b=c", "pass", NONCE.into()).unwrap();
        assert_eq!(scram.first(), format!("n,,n=a=2Cb=3Dc,r={NONCE}"));
    }
}
