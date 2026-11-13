pub const PREF_FILE: &str = "setu-autoconfig.js";
pub const CFG_FILE: &str = "setu.cfg";

pub const PREF: &str = "pref(\"general.config.filename\", \"setu.cfg\");\npref(\"general.config.obscure_value\", 0);\npref(\"general.config.sandbox_enabled\", false);\n";

pub const SCRIPT: &str = r#"//
(function () {
  let prof;
  try {
    prof = Services.dirsvc.get("ProfD", Components.interfaces.nsIFile);
  } catch (e) {
    return;
  }
  const mark = prof.clone();
  mark.append(".setu-profile");
  if (!mark.exists()) return;
  function setuMain() {
    const Cc = Components.classes, Ci = Components.interfaces;
    const Services = globalThis.Services || ChromeUtils.importESModule("resource://gre/modules/Services.sys.mjs").Services;
    const prof = Services.dirsvc.get("ProfD", Ci.nsIFile);
    const lazy = {};
    function addons() {
      if (!lazy.am) lazy.am = ChromeUtils.importESModule("resource://gre/modules/AddonManager.sys.mjs").AddonManager;
      return lazy.am;
    }
    function browserWindow() {
      return Services.wm.getMostRecentWindow("navigator:browser");
    }
    const blockedPrefs = /^(general\.config\.|autoadmin\.|remote\.|devtools\.debugger\.remote|security\.sandbox\.)/;
    const commandIds = /^(cmd_|Browser:|View:|Tools:|History:)[A-Za-z0-9_:.-]{1,60}$/;
    async function run(op, args) {
      switch (op) {
        case "hello":
          return { version: Services.appinfo.version, profile: prof.path };
        case "addons.list": {
          const all = await addons().getAllAddons();
          return all.filter(a => a.type === "extension" || a.type === "theme").map(a => ({ id: a.id, name: a.name, type: a.type, version: a.version, enabled: a.isActive, builtin: !!a.isBuiltin }));
        }
        case "addons.enable":
        case "addons.disable":
        case "addons.remove": {
          const a = await addons().getAddonByID(String(args.id || ""));
          if (!a) throw new Error("No such add-on.");
          if (op === "addons.enable") await a.enable();
          else if (op === "addons.disable") await a.disable();
          else await a.uninstall();
          return { id: a.id, name: a.name };
        }
        case "addons.install": {
          const url = String(args.url || "");
          if (!/^https:\/\/addons\.mozilla\.org\/[A-Za-z0-9_\/.%?=&-]+$/.test(url)) throw new Error("Add-ons come only from addons.mozilla.org.");
          const install = await addons().getInstallForURL(url, { telemetryInfo: { source: "setu" } });
          const a = await install.install();
          return { id: a.id, name: a.name };
        }
        case "prefs.get": {
          const name = String(args.name || "");
          const t = Services.prefs.getPrefType(name);
          if (t === Services.prefs.PREF_BOOL) return Services.prefs.getBoolPref(name);
          if (t === Services.prefs.PREF_INT) return Services.prefs.getIntPref(name);
          if (t === Services.prefs.PREF_STRING) return Services.prefs.getStringPref(name);
          return null;
        }
        case "prefs.set": {
          const name = String(args.name || "");
          if (!name || blockedPrefs.test(name)) throw new Error("That setting is not changed from here.");
          const v = args.value;
          if (typeof v === "boolean") Services.prefs.setBoolPref(name, v);
          else if (typeof v === "number" && Number.isInteger(v)) Services.prefs.setIntPref(name, v);
          else Services.prefs.setStringPref(name, String(v));
          return true;
        }
        case "devtools.open": {
          const win = browserWindow();
          if (!win) throw new Error("No Firefox window is open.");
          const { DevToolsShim } = ChromeUtils.importESModule("chrome://devtools-startup/content/DevToolsShim.sys.mjs");
          await DevToolsShim.showToolboxForTab(win.gBrowser.selectedTab, { toolId: String(args.tool || "webconsole") });
          return true;
        }
        case "command": {
          const id = String(args.id || "");
          if (!commandIds.test(id)) throw new Error("That command is not run from here.");
          const win = browserWindow();
          const el = win && win.document.getElementById(id);
          if (!el) throw new Error("Firefox has no such command.");
          el.doCommand();
          return true;
        }
        default:
          throw new Error("Unknown request.");
      }
    }
    function start() {
      const token = Services.uuid.generateUUID().toString().replace(/[{}-]/g, "");
      const server = Cc["@mozilla.org/network/server-socket;1"].createInstance(Ci.nsIServerSocket);
      server.init(-1, true, 4);
      const info = prof.clone();
      info.append(".setu-autoconfig");
      if (info.exists()) info.remove(false);
      const out = Cc["@mozilla.org/network/file-output-stream;1"].createInstance(Ci.nsIFileOutputStream);
      out.init(info, 0x02 | 0x08 | 0x20, 0o600, 0);
      const text = JSON.stringify({ port: server.port, token: token, pid: Services.appinfo.processID });
      out.write(text, text.length);
      out.close();
      const enc = new TextEncoder(), dec = new TextDecoder();
      server.asyncListen({
        onSocketAccepted(srv, transport) {
          const input = transport.openInputStream(0, 0, 0);
          const output = transport.openOutputStream(1, 0, 0);
          let buf = "", trusted = false;
          const send = obj => {
            const bytes = enc.encode(JSON.stringify(obj) + "\n");
            const s = String.fromCharCode.apply(null, bytes);
            try { output.write(s, s.length); } catch (e) {}
          };
          const handle = line => {
            let msg;
            try { msg = JSON.parse(line); } catch (e) { transport.close(0); return; }
            if (!trusted) {
              if (msg.token !== token) { transport.close(0); return; }
              trusted = true;
            }
            Promise.resolve().then(() => run(String(msg.op || ""), msg.args || {}))
              .then(result => send({ id: msg.id, ok: true, result: result === undefined ? null : result }))
              .catch(e => send({ id: msg.id, ok: false, error: String((e && e.message) || e) }));
          };
          const pump = Cc["@mozilla.org/network/input-stream-pump;1"].createInstance(Ci.nsIInputStreamPump);
          pump.init(input, 0, 0, false);
          pump.asyncRead({
            onStartRequest() {},
            onStopRequest() { try { transport.close(0); } catch (e) {} },
            onDataAvailable(req, stream, offset, count) {
              const sis = Cc["@mozilla.org/scriptableinputstream;1"].createInstance(Ci.nsIScriptableInputStream);
              sis.init(stream);
              const raw = sis.readBytes(count);
              buf += dec.decode(Uint8Array.from(raw, c => c.charCodeAt(0)), { stream: true });
              let i;
              while ((i = buf.indexOf("\n")) >= 0) {
                const line = buf.slice(0, i);
                buf = buf.slice(i + 1);
                if (line.trim()) handle(line);
              }
            }
          });
        },
        onStopListening() {}
      });
      Services.obs.addObserver(() => { try { server.close(); } catch (e) {} if (info.exists()) info.remove(false); }, "quit-application");
    }
    Services.obs.addObserver(function once() {
      Services.obs.removeObserver(once, "final-ui-startup");
      try { start(); } catch (e) { Components.utils.reportError(e); }
    }, "final-ui-startup");
  }
  const sb = Components.utils.Sandbox(Services.scriptSecurityManager.getSystemPrincipal(), { sandboxName: "setu", wantComponents: true, wantGlobalProperties: ["TextEncoder", "TextDecoder", "URL", "ChromeUtils"], freshCompartment: true });
  Components.utils.evalInSandbox("(" + setuMain.toString() + ")()", sb);
})();
"#;
