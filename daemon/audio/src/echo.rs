use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub const PREFIX: &str = "setu-echo";

pub struct Canceller {
    child: Child,
    pub source: String,
    pub mic: String,
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.chars().filter(|c| !matches!(c, '"' | '\\' | '\n' | '\r' | '{' | '}' | '[' | ']')).collect::<String>())
}

pub fn conf(source: &str, mic: &str) -> String {
    let target = if mic.is_empty() { String::new() } else { format!(" target.object = {}", quoted(mic)) };
    format!(
        "context.properties = {{ log.level = 0 }}\n\
context.spa-libs = {{\n    audio.convert.* = audioconvert/libspa-audioconvert\n    support.* = support/libspa-support\n}}\n\
context.modules = [\n\
    {{ name = libpipewire-module-rt flags = [ ifexists nofail ] }}\n\
    {{ name = libpipewire-module-protocol-native }}\n\
    {{ name = libpipewire-module-client-node }}\n\
    {{ name = libpipewire-module-adapter }}\n\
    {{ name = libpipewire-module-echo-cancel\n\
        args = {{\n\
            library.name = aec/libspa-aec-webrtc\n\
            monitor.mode = true\n\
            audio.channels = 1\n\
            aec.args = {{ webrtc.gain_control = false webrtc.extended_filter = true webrtc.noise_suppression = false webrtc.high_pass_filter = true }}\n\
            capture.props = {{ node.name = {cap} node.description = \"Setu echo canceller input\" node.dont-reconnect = false{target} }}\n\
            source.props = {{ node.name = {src} node.description = \"Setu microphone without speaker sound\" priority.session = 0 priority.driver = 0 }}\n\
            playback.props = {{ node.name = {mon} node.description = \"Setu echo reference\" }}\n\
        }}\n\
    }}\n\
]\n",
        cap = quoted(&format!("{source}-capture")),
        src = quoted(source),
        mon = quoted(&format!("{source}-reference")),
    )
}

fn pipewire_server() -> bool {
    Command::new("pactl").arg("info").stdin(Stdio::null()).stderr(Stdio::null()).output().ok().is_some_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("PipeWire"))
}

fn has_source(name: &str) -> bool {
    Command::new("pactl")
        .args(["list", "short", "sources"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.split('\t').nth(1) == Some(name)))
}

fn folder() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("setu")
}

impl Canceller {
    pub fn start(mic: &str) -> Result<Self, String> {
        if !pipewire_server() {
            return Err("the sound server isn't PipeWire".into());
        }
        let source = format!("{PREFIX}-{}", std::process::id());
        let dir = folder();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("{source}.conf"));
        std::fs::write(&path, conf(&source, mic)).map_err(|e| e.to_string())?;
        let mut cmd = Command::new("pipewire");
        cmd.arg("-c").arg(&path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).env_remove("PIPEWIRE_DEBUG");
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let mut child = cmd.spawn().map_err(|e| format!("pipewire could not start: {e}"))?;
        let until = Instant::now() + Duration::from_secs(4);
        while Instant::now() < until {
            if let Ok(Some(st)) = child.try_wait() {
                let _ = std::fs::remove_file(&path);
                return Err(format!("the echo canceller stopped ({st})"));
            }
            if has_source(&source) {
                let _ = std::fs::remove_file(&path);
                return Ok(Self { child, source, mic: mic.to_owned() });
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&path);
        Err("the echo canceller did not appear".into())
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Canceller {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_cancels_what_the_speakers_play_and_never_becomes_the_default() {
        let c = conf("setu-echo-7", "alsa_input.usb-mic");
        assert!(c.contains("monitor.mode = true") && c.contains("aec/libspa-aec-webrtc") && c.contains("webrtc.noise_suppression = false"));
        assert!(c.contains("node.name = \"setu-echo-7\"") && c.contains("priority.session = 0"));
        assert!(c.contains("target.object = \"alsa_input.usb-mic\""));
        assert!(!conf("setu-echo-7", "").contains("target.object"));
        assert!(!conf("x", "a\" } ] evil = {").contains("evil = {\""));
    }
}
