use crate::core::Core;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::{Value, json};

pub const URL: Option<&str> = option_env!("SETU_UPDATE_URL");
pub const KEY: Option<&str> = option_env!("SETU_UPDATE_PUBKEY");

pub fn newer(a: &str, b: &str) -> bool {
    let p = |s: &str| s.split(['.', '-']).take(3).map(|x| x.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    p(a) > p(b)
}

pub fn state(core: &Core, found: Option<&Value>, failed: bool) -> Value {
    let channel = core.settings.str("update.channel");
    json!({
        "configured": URL.is_some() && KEY.is_some(),
        "available": found.is_some_and(|f| f["version"].as_str().is_some_and(|v| newer(v, setu_common::VERSION))),
        "version": found.and_then(|f| f["version"].as_str()),
        "current": setu_common::VERSION,
        "channel": channel,
        "progress": null,
        "failed": failed,
        "failedVersion": null,
        "notes": found.and_then(|f| f["notes"].as_str()),
        "justUpdated": false,
    })
}

pub fn verify(body: &[u8], sig_b64: &str) -> bool {
    use base64::Engine;
    let Some(k) = KEY else { return false };
    let (Ok(kb), Ok(sb)) = (base64::engine::general_purpose::STANDARD.decode(k), base64::engine::general_purpose::STANDARD.decode(sig_b64.trim())) else { return false };
    let (Ok(arr), Ok(sig)) = (<[u8; 32]>::try_from(kb.as_slice()), Signature::from_slice(&sb)) else { return false };
    VerifyingKey::from_bytes(&arr).is_ok_and(|vk| vk.verify(body, &sig).is_ok())
}

pub async fn check(core: &Core) -> Value {
    let Some(base) = URL else { return state(core, None, false) };
    let channel = core.settings.str("update.channel");
    let url = format!("{}/{}/{}-{}.json", base.trim_end_matches('/'), channel, std::env::consts::OS, std::env::consts::ARCH);
    let r = crate::work::background("update", move || -> anyhow::Result<Value> {
        let agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(20))).build().new_agent();
        let manifest = agent.get(&url).call()?.body_mut().read_to_vec()?;
        let sig = agent.get(&format!("{url}.sig")).call()?.body_mut().read_to_string()?;
        if !verify(&manifest, &sig) {
            anyhow::bail!("update manifest signature is invalid");
        }
        Ok(serde_json::from_slice(&manifest)?)
    })
    .await;
    match r {
        Ok(Ok(v)) => state(core, Some(&v), false),
        Ok(Err(e)) => {
            log::warn!("update check failed: {e}");
            state(core, None, false)
        }
        Err(_) => state(core, None, false),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn compares_versions() {
        assert!(super::newer("1.0.1", "1.0.0"));
        assert!(super::newer("1.10.0", "1.9.9"));
        assert!(!super::newer("1.0.0", "1.0.0"));
        assert!(!super::verify(b"x", "AAAA"));
    }
}
