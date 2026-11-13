use crate::bidi::Bidi;
use crate::cdp::{Cdp, Tab};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub enum Conn {
    Cdp(Arc<Cdp>),
    Bidi(Arc<Bidi>),
}

pub fn any() -> Option<Conn> {
    crate::cdp::any().map(Conn::Cdp).or_else(|| crate::bidi::any().map(Conn::Bidi))
}

pub fn connected(key: &str) -> Option<Conn> {
    crate::cdp::connected(key).map(Conn::Cdp).or_else(|| crate::bidi::connected(key).map(Conn::Bidi))
}

pub fn supported(key: &str) -> bool {
    matches!(crate::tools::browsers::family(key), "chromium" | "firefox")
}

pub async fn ensure(data: &Path, key: &str, exe: &Path, url: Option<&str>) -> Result<(Conn, bool)> {
    if let Some(c) = connected(key) {
        return Ok((c, false));
    }
    let dir = crate::cdp::profile_dir(data, key);
    match crate::tools::browsers::family(key) {
        "firefox" => crate::bidi::launch(key, exe, &dir, url).await.map(|c| (Conn::Bidi(c), true)),
        "chromium" => crate::cdp::launch(key, exe, &dir, &crate::cdp::flags(), url).await.map(|c| (Conn::Cdp(c), true)),
        _ => Err(anyhow!("Full control of that browser isn't available.")),
    }
}

impl Conn {
    pub async fn tabs(&self) -> Result<Vec<Tab>> {
        match self {
            Conn::Cdp(c) => c.tabs().await,
            Conn::Bidi(c) => c.tabs().await,
        }
    }

    pub async fn open(&self, url: &str) -> Result<String> {
        match self {
            Conn::Cdp(c) => c.open(url).await,
            Conn::Bidi(c) => c.open(url).await,
        }
    }

    pub async fn activate(&self, id: &str) -> Result<()> {
        match self {
            Conn::Cdp(c) => c.activate(id).await,
            Conn::Bidi(c) => c.activate(id).await,
        }
    }

    pub async fn close(&self, id: &str) -> Result<()> {
        match self {
            Conn::Cdp(c) => c.close(id).await,
            Conn::Bidi(c) => c.close(id).await,
        }
    }

    pub async fn navigate(&self, id: &str, url: &str) -> Result<()> {
        match self {
            Conn::Cdp(c) => c.navigate(id, url).await,
            Conn::Bidi(c) => c.navigate(id, url).await,
        }
    }

    pub async fn pause_media(&self, id: &str) -> Result<usize> {
        match self {
            Conn::Cdp(c) => c.pause_media(id).await,
            Conn::Bidi(c) => c.pause_media(id).await,
        }
    }

    pub async fn capture(&self, id: &str, pdf: bool) -> Result<Vec<u8>> {
        match self {
            Conn::Cdp(c) => c.capture(id, pdf).await,
            Conn::Bidi(c) => c.capture(id, pdf).await,
        }
    }

    pub async fn page_text(&self, id: &str, max: usize) -> Result<(String, String)> {
        match self {
            Conn::Cdp(c) => c.page_text(id, max).await,
            Conn::Bidi(c) => c.page_text(id, max).await,
        }
    }

    pub async fn console_errors(&self, id: &str) -> Result<Vec<String>> {
        match self {
            Conn::Cdp(c) => c.console_errors(id).await,
            Conn::Bidi(c) => c.console_errors(id).await,
        }
    }

    pub async fn eval_json(&self, id: &str, expression: &str) -> Result<Value> {
        let text = match self {
            Conn::Cdp(c) => {
                let s = c.session(id).await?;
                let r = c.call("Runtime.evaluate", json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }), Some(&s)).await?;
                if let Some(e) = r.pointer("/exceptionDetails/text").and_then(Value::as_str) {
                    return Err(anyhow!("The page refused that: {e}"));
                }
                r.pointer("/result/value").and_then(Value::as_str).unwrap_or("null").to_owned()
            }
            Conn::Bidi(c) => c.eval(id, expression).await?.as_str().unwrap_or("null").to_owned(),
        };
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    pub async fn failed_requests(&self, id: &str) -> Result<Vec<(String, u64)>> {
        let v = self.eval_json(id, NETWORK_JS).await?;
        Ok(v.as_array().map(|a| a.iter().filter_map(|x| Some((x.get(0)?.as_str()?.to_owned(), x.get(1)?.as_u64()?))).collect()).unwrap_or_default())
    }

    pub async fn set_permission(&self, name: &str, state: &str, origin: &str) -> Result<()> {
        match self {
            Conn::Cdp(c) => c.call("Browser.setPermission", json!({ "permission": { "name": name }, "setting": state, "origin": origin }), None).await.map(|_| ()),
            Conn::Bidi(c) => c.call("permissions.setPermission", json!({ "descriptor": { "name": name }, "state": state, "origin": origin })).await.map(|_| ()),
        }
    }

    pub fn alive(&self) -> bool {
        match self {
            Conn::Cdp(c) => crate::cdp::connected(&c.key).is_some(),
            Conn::Bidi(c) => crate::bidi::connected(&c.key).is_some(),
        }
    }

    pub async fn shutdown(&self) {
        let _ = match self {
            Conn::Cdp(c) => c.call("Browser.close", json!({}), None).await,
            Conn::Bidi(c) => c.call("browser.close", json!({})).await,
        };
        for _ in 0..100 {
            if !self.alive() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    pub async fn new_window(&self) -> Result<()> {
        match self {
            Conn::Cdp(c) => {
                let r = c.call("Target.createTarget", json!({ "url": "about:blank", "newWindow": true }), None).await?;
                if let Some(id) = r.get("targetId").and_then(Value::as_str) {
                    let _ = c.activate(id).await;
                }
                Ok(())
            }
            Conn::Bidi(c) => {
                let r = c.call("browsingContext.create", json!({ "type": "window" })).await?;
                if let Some(id) = r.get("context").and_then(Value::as_str) {
                    let _ = c.activate(id).await;
                }
                Ok(())
            }
        }
    }
}

const NETWORK_JS: &str = "JSON.stringify((()=>{const o=[];const p=u=>{try{const x=new URL(u,location.href);return x.host===location.host?x.pathname:x.host+x.pathname}catch(e){return u}};const n=performance.getEntriesByType('navigation')[0];if(n&&n.responseStatus>=400)o.push([p(location.href),n.responseStatus]);for(const e of performance.getEntriesByType('resource')){if(e.responseStatus>=400)o.push([p(e.name),e.responseStatus])}return o.slice(0,12)})())";

pub const FORM_LIST_JS: &str = "JSON.stringify((()=>{const vis=e=>{const r=e.getBoundingClientRect();const s=getComputedStyle(e);return r.width>0&&r.height>0&&s.visibility!=='hidden'&&s.display!=='none'};const a=document.activeElement;const root=(a&&a.form)||document.querySelector('form')||document;const skip=['hidden','submit','button','reset','image','file'];const els=[...root.querySelectorAll('input,textarea,select')].filter(e=>!skip.includes((e.type||'').toLowerCase())&&vis(e));const label=e=>{let t='';if(e.id){const l=document.querySelector('label[for=\"'+CSS.escape(e.id)+'\"]');if(l)t=l.innerText}if(!t&&e.closest('label'))t=e.closest('label').innerText;t=t||e.getAttribute('aria-label')||e.placeholder||e.name||e.id||e.type||'';return t.replace(/\\s+/g,' ').trim().replace(/[*:]+$/,'').trim()};window.__setuFields=els;return els.map((e,i)=>({i,label:label(e),type:(e.type||e.tagName).toLowerCase(),required:!!e.required,filled:(e.type==='checkbox'||e.type==='radio')?e.checked:!!e.value,options:e.tagName==='SELECT'?[...e.options].map(o=>o.text.trim()):[]}))})())";

pub fn form_fill_js(i: usize, value: &str) -> String {
    let v = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into());
    format!("JSON.stringify(((i,v)=>{{const e=(window.__setuFields||[])[i];if(!e)return {{ok:false,why:'gone'}};e.focus();if(e.type==='checkbox'||e.type==='radio'){{const want=!/^(no|false|off|untick|uncheck)/i.test(v);if(e.checked!==want)e.click();return {{ok:true}}}}if(e.tagName==='SELECT'){{const o=[...e.options].find(o=>o.text.toLowerCase().includes(v.toLowerCase()));if(!o)return {{ok:false,why:'choice'}};e.value=o.value}}else{{const pr=e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(pr,'value').set.call(e,v)}}e.dispatchEvent(new Event('input',{{bubbles:true}}));e.dispatchEvent(new Event('change',{{bubbles:true}}));return {{ok:true}}}})({i},{v}))")
}

pub const FORM_SUBMIT_JS: &str = "JSON.stringify((()=>{const e=(window.__setuFields||[])[0];const f=(e&&e.form)||document.querySelector('form');if(!f)return {ok:false};if(f.requestSubmit)f.requestSubmit();else f.submit();return {ok:true}})())";

pub async fn active_tab(c: &Conn) -> Result<Tab> {
    let tabs = c.tabs().await?;
    if tabs.len() == 1 {
        return Ok(tabs[0].clone());
    }
    let front = crate::os::front().await.ok().map(|w| w.title).unwrap_or_default();
    let title = front.rsplit_once(" — ").or_else(|| front.rsplit_once(" - ")).map(|(t, _)| t.trim().to_owned()).unwrap_or(front);
    tabs.iter()
        .find(|t| !title.is_empty() && t.title == title)
        .or_else(|| tabs.iter().find(|t| !title.is_empty() && !t.title.is_empty() && (title.starts_with(&t.title) || t.title.starts_with(&title))))
        .or_else(|| tabs.first())
        .cloned()
        .ok_or_else(|| anyhow!("The browser has no open tab."))
}
