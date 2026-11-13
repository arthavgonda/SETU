use crate::control::{ControlMsg, Req};
use crate::core::Core;
use crate::hub::Frame;
use serde_json::{Value, json};
use setu_common::frame::{MAX_FRAME, parse_object};
use setu_common::protocol::{ApiError, ErrorCode, Incoming, ParamsExt, parse_envelope, response_err, response_ok};
use setu_common::{PROTOCOL_MAX, PROTOCOL_MIN};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

async fn read_frame<R: AsyncRead + Unpin>(r: &mut R, buf: &mut Vec<u8>) -> Result<(), String> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await.map_err(|e| e.to_string())?;
    let n = u32::from_be_bytes(len) as usize;
    if n == 0 || n > MAX_FRAME {
        return Err(format!("bad frame length {n}"));
    }
    buf.clear();
    buf.resize(n, 0);
    r.read_exact(buf).await.map_err(|e| e.to_string())?;
    Ok(())
}

fn frame(v: &Value) -> Option<Frame> {
    let body = serde_json::to_vec(v).ok()?;
    setu_common::frame::encode(&body).ok().map(Arc::new)
}

pub async fn snapshot(core: &Arc<Core>) -> Value {
    let (tx, rx) = oneshot::channel();
    let (session, mic) = if core.control.send(ControlMsg::Req(Req::Snapshot { reply: tx })).await.is_ok() {
        rx.await.unwrap_or((Value::Null, Value::Null))
    } else {
        (Value::Null, Value::Null)
    };
    json!({
        "daemon": core.daemon_state(),
        "mic": mic,
        "session": session,
        "attention": core.attention.list(),
        "settings": core.settings.all_public(),
        "models": crate::handlers::slots_json(core),
        "update": crate::update::state(core, None, false),
        "camera": { "open": false, "secondsLeft": null },
        "speaker": { "enrolled": crate::speaker::enrolled(&core.db) },
        "manifestVersion": core.store.manifest.version,
        "paths": core.paths.to_json(),
    })
}

pub async fn serve_conn<S>(core: Arc<Core>, stream: S)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut rd, mut wr) = tokio::io::split(stream);
    let (tx, mut rx) = mpsc::channel::<Frame>(512);
    let (kill_tx, mut kill_rx) = oneshot::channel::<()>();
    let id = core.hub.register(tx.clone(), kill_tx);
    log::debug!("client {id} connected");
    let writer = tokio::spawn(async move {
        while let Some(f) = rx.recv().await {
            if wr.write_all(&f).await.is_err() {
                break;
            }
        }
        let _ = wr.shutdown().await;
    });
    let mut hello = false;
    let mut buf = Vec::new();
    loop {
        let r = tokio::select! {
            r = read_frame(&mut rd, &mut buf) => r,
            _ = &mut kill_rx => Err("closed by daemon".into()),
        };
        if let Err(e) = r {
            if !e.contains("early eof") {
                log::debug!("client {id} disconnected: {e}");
            }
            break;
        }
        let msg = match parse_object(&buf).map_err(|e| e.to_string()).and_then(|m| parse_envelope(m).map_err(|e| e.message)) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("client {id} sent a malformed frame: {e}");
                break;
            }
        };
        let Incoming::Request { id: rid, method, params } = msg else { continue };
        if !hello && method != "hello" {
            if let Some(f) = frame(&response_err(&rid, &ApiError::bad("hello must be the first request"))) {
                let _ = tx.send(f).await;
            }
            break;
        }
        match method.as_str() {
            "hello" => {
                let proto = params.opt_i64("protocol").unwrap_or(0) as u32;
                if !(PROTOCOL_MIN..=PROTOCOL_MAX).contains(&proto) {
                    let e = ApiError::new(ErrorCode::VersionMismatch, "This version of the app doesn't match the background service.").with_details(json!({ "min": PROTOCOL_MIN, "max": PROTOCOL_MAX }));
                    if let Some(f) = frame(&response_err(&rid, &e)) {
                        let _ = tx.send(f).await;
                    }
                    break;
                }
                let role = params.opt_str("role").filter(|r| *r == "shell" || *r == "window").unwrap_or("window");
                log::debug!("client {id} is {} {} as {role}", params.opt_str("client").unwrap_or("unknown"), params.opt_str("version").unwrap_or("?"));
                core.hub.set_role(id, role);
                hello = true;
                let snap = snapshot(&core).await;
                let res = json!({ "daemon": { "version": setu_common::VERSION, "build": env!("SETU_BUILD") }, "protocol": { "min": PROTOCOL_MIN, "max": PROTOCOL_MAX }, "snapshot": snap });
                if let Some(f) = frame(&response_ok(&rid, res)) {
                    let _ = tx.send(f).await;
                }
            }
            "events.subscribe" | "events.unsubscribe" => {
                let topics = params.str_list("topics");
                let now = if method == "events.subscribe" { core.hub.subscribe(id, &topics) } else { core.hub.unsubscribe(id, &topics) };
                if let Some(f) = frame(&response_ok(&rid, json!({ "topics": now }))) {
                    let _ = tx.send(f).await;
                }
            }
            _ => {
                let core = core.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let r = crate::handlers::dispatch(&core, &method, params).await;
                    let v = match r {
                        Ok(v) => response_ok(&rid, v),
                        Err(e) => {
                            if e.code == ErrorCode::Internal {
                                log::error!("{method} failed: {}", e.message);
                            }
                            response_err(&rid, &e)
                        }
                    };
                    match frame(&v) {
                        Some(f) => {
                            let _ = tx.send(f).await;
                        }
                        None => {
                            if let Some(f) = frame(&response_err(&rid, &ApiError::internal("The answer was too large to send."))) {
                                let _ = tx.send(f).await;
                            }
                        }
                    }
                });
            }
        }
    }
    core.hub.unregister(id);
    drop(tx);
    let _ = writer.await;
}

#[cfg(unix)]
pub async fn serve(core: Arc<Core>) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = core.paths.socket();
    if path.exists() {
        if tokio::net::UnixStream::connect(&path).await.is_ok() {
            anyhow::bail!("another Setu service is already running");
        }
        std::fs::remove_file(&path)?;
    }
    let old = unsafe { libc::umask(0o177) };
    let listener = tokio::net::UnixListener::bind(&path);
    unsafe { libc::umask(old) };
    let listener = listener?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    log::info!("listening on {}", path.display());
    let me = crate::paths::uid();
    loop {
        let (stream, _) = listener.accept().await?;
        match stream.peer_cred() {
            Ok(c) if c.uid() == me => {
                tokio::spawn(serve_conn(core.clone(), stream));
            }
            Ok(c) => log::warn!("refused a connection from user {}", c.uid()),
            Err(e) => log::warn!("refused a connection without credentials: {e}"),
        }
    }
}

#[cfg(windows)]
pub async fn serve(core: Arc<Core>) -> anyhow::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    let name = format!(r"\\.\pipe\setu-{user}");
    let mut server = ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(&name)?;
    loop {
        server.connect().await?;
        let conn = server;
        server = ServerOptions::new().reject_remote_clients(true).create(&name)?;
        tokio::spawn(serve_conn(core.clone(), conn));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_oversized_and_empty_frames() {
        let mut big = std::io::Cursor::new(((MAX_FRAME + 1) as u32).to_be_bytes().to_vec());
        assert!(read_frame(&mut big, &mut Vec::new()).await.is_err());
        let mut zero = std::io::Cursor::new(0u32.to_be_bytes().to_vec());
        assert!(read_frame(&mut zero, &mut Vec::new()).await.is_err());
        let mut ok = std::io::Cursor::new(setu_common::frame::encode(b"{}").unwrap());
        let mut b = Vec::new();
        read_frame(&mut ok, &mut b).await.unwrap();
        assert_eq!(b, b"{}");
    }
}
