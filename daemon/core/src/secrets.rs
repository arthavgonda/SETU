use anyhow::{Result, anyhow};

pub fn valid_ref(r: &str) -> bool {
    !r.is_empty() && r.len() <= 64 && r.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

#[cfg(target_os = "linux")]
mod ss {
    use anyhow::{Result, anyhow};
    use futures_util::StreamExt;
    use std::collections::HashMap;
    use std::time::Duration;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    pub const DEST: &str = "org.freedesktop.secrets";

    pub async fn service(c: &zbus::Connection) -> Result<zbus::Proxy<'static>> {
        zbus::Proxy::new(c, DEST, "/org/freedesktop/secrets", "org.freedesktop.Secret.Service").await.map_err(|_| anyhow!("Your system has no keychain service, so Setu can't keep secrets safely."))
    }

    pub async fn prompt(c: &zbus::Connection, path: &OwnedObjectPath) -> Result<OwnedValue> {
        let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.Secret.Prompt")?.member("Completed")?.path(path.as_str())?.build();
        let mut done = zbus::MessageStream::for_match_rule(rule, c, Some(4)).await?;
        c.call_method(Some(DEST), path.as_str(), Some("org.freedesktop.Secret.Prompt"), "Prompt", &("",)).await?;
        let msg = tokio::time::timeout(Duration::from_secs(120), done.next()).await.map_err(|_| anyhow!("The keychain didn't open in time."))?.ok_or_else(|| anyhow!("The keychain didn't answer."))??;
        let (dismissed, result): (bool, OwnedValue) = msg.body().deserialize()?;
        if dismissed {
            return Err(anyhow!("The keychain stayed locked."));
        }
        Ok(result)
    }

    pub async fn unlock(c: &zbus::Connection, svc: &zbus::Proxy<'_>, objs: Vec<OwnedObjectPath>) -> Result<()> {
        let (_, p): (Vec<OwnedObjectPath>, OwnedObjectPath) = svc.call("Unlock", &(objs,)).await?;
        if p.as_str() != "/" {
            prompt(c, &p).await?;
        }
        Ok(())
    }

    pub async fn search(c: &zbus::Connection, svc: &zbus::Proxy<'_>, r: &str) -> Result<Vec<OwnedObjectPath>> {
        let attrs: HashMap<&str, &str> = HashMap::from([("application", "setu"), ("ref", r)]);
        let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = svc.call("SearchItems", &(attrs.clone(),)).await?;
        if !unlocked.is_empty() || locked.is_empty() {
            return Ok(unlocked);
        }
        unlock(c, svc, locked).await?;
        let (unlocked, _): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = svc.call("SearchItems", &(attrs,)).await?;
        Ok(unlocked)
    }

    pub async fn session(svc: &zbus::Proxy<'_>) -> Result<OwnedObjectPath> {
        let (_, s): (OwnedValue, OwnedObjectPath) = svc.call("OpenSession", &("plain", Value::from(""))).await?;
        Ok(s)
    }

    pub async fn close(c: &zbus::Connection, s: &OwnedObjectPath) {
        let _ = c.call_method(Some(DEST), s.as_str(), Some("org.freedesktop.Secret.Session"), "Close", &()).await;
    }
}

#[cfg(target_os = "linux")]
pub async fn read_bytes(r: &str) -> Result<Vec<u8>> {
    use std::collections::HashMap;
    use zbus::zvariant::OwnedObjectPath;
    if !valid_ref(r) {
        return Err(anyhow!("invalid key reference"));
    }
    let c = zbus::Connection::session().await?;
    let svc = ss::service(&c).await?;
    let items = ss::search(&c, &svc, r).await?;
    let Some(item) = items.first().cloned() else { return Err(anyhow!("No key is saved for {r}.")) };
    let session = ss::session(&svc).await?;
    type Secrets = HashMap<OwnedObjectPath, (OwnedObjectPath, Vec<u8>, Vec<u8>, String)>;
    let got: Result<Secrets> = svc.call("GetSecrets", &(vec![item], session.clone())).await.map_err(Into::into);
    ss::close(&c, &session).await;
    let (_, (_, _, value, _)) = got?.into_iter().next().ok_or_else(|| anyhow!("The key could not be read."))?;
    Ok(value)
}

#[cfg(target_os = "linux")]
pub async fn write(r: &str, label: &str, value: &[u8]) -> Result<()> {
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedObjectPath, Value};
    if !valid_ref(r) {
        return Err(anyhow!("invalid key reference"));
    }
    let c = zbus::Connection::session().await?;
    let svc = ss::service(&c).await?;
    let mut coll: OwnedObjectPath = svc.call("ReadAlias", &("default",)).await?;
    if coll.as_str() == "/" {
        let props: HashMap<&str, Value> = HashMap::from([("org.freedesktop.Secret.Collection.Label", Value::from("Login"))]);
        let (made, p): (OwnedObjectPath, OwnedObjectPath) = svc.call("CreateCollection", &(props, "default")).await?;
        coll = if p.as_str() != "/" { OwnedObjectPath::try_from(ss::prompt(&c, &p).await?).map_err(|_| anyhow!("The keychain wasn't created."))? } else { made };
    }
    ss::unlock(&c, &svc, vec![coll.clone()]).await?;
    let session = ss::session(&svc).await?;
    let attrs: HashMap<&str, &str> = HashMap::from([("application", "setu"), ("ref", r)]);
    let props: HashMap<&str, Value> = HashMap::from([("org.freedesktop.Secret.Item.Label", Value::from(label)), ("org.freedesktop.Secret.Item.Attributes", Value::from(attrs))]);
    let secret = (session.clone(), Vec::<u8>::new(), value.to_vec(), "text/plain");
    let made: Result<(OwnedObjectPath, OwnedObjectPath)> = c.call_method(Some(ss::DEST), coll.as_str(), Some("org.freedesktop.Secret.Collection"), "CreateItem", &(props, secret, true)).await.and_then(|m| m.body().deserialize()).map_err(Into::into);
    ss::close(&c, &session).await;
    let (_, p) = made?;
    if p.as_str() != "/" {
        ss::prompt(&c, &p).await?;
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn entry(r: &str) -> Result<keyring::Entry> {
    #[cfg(target_os = "macos")]
    let e = keyring::Entry::new("ai.setu", r)?;
    #[cfg(windows)]
    let e = keyring::Entry::new_with_target(&format!("ai.setu/{r}"), "ai.setu", r)?;
    Ok(e)
}

#[cfg(not(target_os = "linux"))]
pub async fn read_bytes(r: &str) -> Result<Vec<u8>> {
    if !valid_ref(r) {
        return Err(anyhow!("invalid key reference"));
    }
    let r = r.to_owned();
    crate::layers::blocking(move || {
        entry(&r)?.get_secret().map_err(|e| match e {
            keyring::Error::NoEntry => anyhow!("No key is saved for {r}."),
            e => e.into(),
        })
    })
    .await?
}

#[cfg(not(target_os = "linux"))]
pub async fn write(r: &str, _label: &str, value: &[u8]) -> Result<()> {
    if !valid_ref(r) {
        return Err(anyhow!("invalid key reference"));
    }
    let (r, v) = (r.to_owned(), value.to_vec());
    crate::layers::blocking(move || entry(&r)?.set_secret(&v).map_err(Into::into)).await?
}

pub async fn read(r: &str) -> Result<String> {
    let mut v = read_bytes(r).await?;
    let s = String::from_utf8(v.clone()).map_err(|_| anyhow!("The saved key is not text."));
    v.iter_mut().for_each(|b| *b = 0);
    s
}

pub fn missing(e: &anyhow::Error) -> bool {
    e.to_string().starts_with("No key is saved")
}

#[cfg(test)]
mod tests {
    #[test]
    fn refs_are_restricted() {
        assert!(super::valid_ref("api.llm"));
        assert!(!super::valid_ref("api llm"));
        assert!(!super::valid_ref(""));
        assert!(!super::valid_ref("../etc"));
    }
}
