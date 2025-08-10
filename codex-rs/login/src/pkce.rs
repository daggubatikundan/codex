use base64::Engine as _;
use rand::RngCore;
use sha2::Digest;
use sha2::Sha256;

#[derive(Debug, Clone)]
pub(crate) struct PkceCodes {
    pub(crate) code_verifier: String,
    pub(crate) code_challenge: String,
}

pub(crate) fn generate_pkce() -> PkceCodes {
    let mut bytes = [0u8; 64];
    rand::thread_rng().fill_bytes(&mut bytes);
    let code_verifier = hex::encode(bytes);
    let digest = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    PkceCodes {
        code_verifier,
        code_challenge,
    }
}
