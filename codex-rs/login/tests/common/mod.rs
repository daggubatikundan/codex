#![allow(clippy::unwrap_used, clippy::expect_used)]
pub fn make_fake_jwt(payload: serde_json::Value) -> String {
    use base64::Engine;
    let header = serde_json::json!({"alg": "none", "typ": "JWT"});
    let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
    let header_b64 = b64(&serde_json::to_vec(&header).unwrap());
    let payload_b64 = b64(&serde_json::to_vec(&payload).unwrap());
    let signature_b64 = b64(b"sig");
    format!("{header_b64}.{payload_b64}.{signature_b64}")
}
