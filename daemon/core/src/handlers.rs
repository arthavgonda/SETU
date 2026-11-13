use crate::control::{self, ConfirmSpec, ControlMsg, Req};
use crate::core::Core;
use crate::models::SlotConfig;
use crate::storage::{HISTORY_COLS, fts_query, history_json};
use rusqlite::params_from_iter;
use serde_json::{Map, Value, json};
use setu_common::protocol::{ApiError, ApiResult, ErrorCode, ParamsExt, now_ms};
use std::sync::Arc;

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::internal(e.to_string())
}

pub fn slot_state(core: &Core, slot: &str, status: Option<&str>, message: Option<&str>, ram: u64) -> Value {
    let cfg = core.slot(slot);
    let engine = match slot {
        "asr" => &core.engines.asr,
        "llm" => &core.engines.llm,
        _ => &core.engines.tts,
    };
    let installed = cfg.provider != "local" || cfg.path.as_ref().is_some_and(|p| std::path::Path::new(p).is_file()) || (!cfg.models.is_empty() && cfg.models.iter().all(|m| core.store.is_installed(m)));
    let st = status.unwrap_or(if !installed {
        "missing"
    } else if cfg.provider != "local" || engine.is_running() {
        "loaded"
    } else {
        "unloaded"
    });
    let name = if cfg.provider != "local" {
        cfg.model_name.clone().unwrap_or_default()
    } else if let Some(p) = &cfg.path {
        std::path::Path::new(p).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        cfg.models.iter().filter_map(|m| core.store.manifest.model(m)).map(|m| m.name.clone()).collect::<Vec<_>>().join(" + ")
    };
    let speed = core.bench.lock().unwrap().as_ref().and_then(|b| match slot {
        "asr" => b["asrRtf"].as_f64().map(|v| json!({ "value": v, "unit": "x real time" })),
        "llm" => b["llmTokensPerS"].as_f64().map(|v| json!({ "value": v, "unit": "tokens/s" })),
        _ => None,
    });
    json!({
        "slot": slot,
        "provider": cfg.provider,
        "model": cfg.models.first().cloned().or(cfg.path.clone()),
        "models": cfg.models,
        "name": name,
        "status": st,
        "diskBytes": core.store.disk_for(&cfg),
        "ramBytes": if ram > 0 { ram } else if engine.is_running() { engine.ram() } else { core.store.ram_for(&cfg) },
        "speed": speed,
        "baseUrl": cfg.base_url,
        "modelName": cfg.model_name,
        "keyRef": cfg.key_ref,
        "voice": cfg.voice,
        "device": engine.device(),
        "message": message,
    })
}

pub fn local_slot(core: &Core, slot: &str, id: &str, voice: Option<String>) -> Result<SlotConfig, ApiError> {
    let def = core.store.manifest.model(id).ok_or_else(|| ApiError::not_found("No such model."))?;
    if def.slot != slot {
        return Err(ApiError::invalid("That model belongs to another slot."));
    }
    let mut models = core.store.default_slots(&core.tier(), &core.lang()).remove(slot).map(|s| s.models).unwrap_or_default();
    if slot == "asr" {
        let current = core.slot("asr");
        if current.provider == "local" && !current.models.is_empty() {
            models = current.models;
        }
        let role = def.role.clone();
        models.retain(|m| core.store.manifest.model(m).is_some_and(|x| x.role != role));
        models.push(id.into());
    } else {
        models = vec![id.into()];
    }
    let voice = voice.filter(|v| def.speakers.iter().any(|s| &s.id == v)).or_else(|| {
        let cur = core.slot(slot);
        cur.voice.filter(|v| def.speakers.iter().any(|s| &s.id == v))
    });
    Ok(SlotConfig { provider: "local".into(), models, voice, ..Default::default() })
}

fn apply_slot(core: &Core, slot: &str, cfg: Option<SlotConfig>) -> Value {
    core.set_slot(slot, cfg);
    core.engines.get(setu_common::engine::EngineKind::parse(slot).unwrap()).stop();
    crate::pipeline::reset_prefix();
    let s = slot_state(core, slot, None, None, 0);
    core.hub.emit("models.state", s.clone());
    s
}

async fn paragraph(core: &Core) -> String {
    if slot_state(core, "llm", None, None, 0)["status"] == "missing" {
        return control::fallback_paragraph();
    }
    let system = "Write one short paragraph that a person will read aloud three times so a voice assistant called Setu can learn their voice. \
It must begin with the words Hey Setu, followed by a comma. Use three or four short sentences, about 35 words, of warm, natural, everyday English with common words, \
written in the first person about an ordinary day in India. Do not introduce anyone, and do not use numbers, quotes, emoji, or any name other than Setu. Answer as JSON {\"text\": \"...\"}.";
    for _ in 0..2 {
        match crate::pipeline::ask_json(core, system, "Write a new paragraph.", Some(crate::pipeline::string_grammar("text")), 140, 0.9).await {
            Ok(v) => {
                let t = v["text"].as_str().unwrap_or("");
                if let Some(p) = control::shape_paragraph(t) {
                    return p;
                }
                log::info!("generated paragraph rejected: {}", t.chars().take(160).collect::<String>());
            }
            Err(e) => log::warn!("paragraph generation failed: {e}"),
        }
    }
    control::fallback_paragraph()
}

pub fn finish_pending_use(core: &Core) {
    let ready: Vec<(String, String, Option<String>)> = {
        let mut g = core.pending_use.lock().unwrap();
        let (done, wait): (Vec<_>, Vec<_>) = g.drain(..).partition(|(id, _, _)| core.store.is_installed(id));
        *g = wait;
        done
    };
    for (id, slot, voice) in ready {
        match local_slot(core, &slot, &id, voice) {
            Ok(cfg) => {
                log::info!("switching {slot} to {id} now that it is downloaded");
                apply_slot(core, &slot, Some(cfg));
                core.hub.emit("models.used", json!({ "id": id, "slot": slot }));
            }
            Err(e) => log::warn!("{id} could not be used: {}", e.message),
        }
    }
}

fn use_model(core: &Core, p: &Value) -> ApiResult {
    let id = p.str_field("id")?;
    let def = core.store.manifest.model(id).ok_or_else(|| ApiError::not_found("No such model."))?;
    let slot = def.slot.clone();
    if !["asr", "llm", "tts"].contains(&slot.as_str()) {
        return Err(ApiError::invalid("That model can't be chosen."));
    }
    let voice = p.opt_str("voice").filter(|v| !v.is_empty()).map(str::to_owned);
    let total = crate::models::total_ram();
    let (fits, _) = crate::models::fit(&core.store.manifest, id, total, crate::gpu::best().as_ref());
    if !fits && !p.opt_bool("force").unwrap_or(false) {
        return Err(ApiError::invalid(if def.gpu.as_deref() == Some("needed") {
            "Oops, this voice needs a graphics card. Try Kokoro instead."
        } else {
            "Oops, this model is too much for this computer. Try a smaller one, or an online model."
        }));
    }
    if core.store.is_installed(id) {
        let cfg = local_slot(core, &slot, id, voice)?;
        let s = apply_slot(core, &slot, Some(cfg));
        return Ok(json!({ "state": s, "queued": [] }));
    }
    let ids = core.store.manifest.with_requires(&[id.to_owned()]);
    let r = core.downloads.enqueue(&ids)?;
    {
        let mut g = core.pending_use.lock().unwrap();
        g.retain(|(_, s, _)| s != &slot);
        g.push((id.to_owned(), slot.clone(), voice));
    }
    Ok(json!({ "state": slot_state(core, &slot, None, None, 0), "queued": r["queued"] }))
}

pub fn slots_json(core: &Core) -> Vec<Value> {
    ["asr", "llm", "tts"].iter().map(|s| slot_state(core, s, None, None, 0)).collect()
}

fn tiers_json(core: &Core) -> Value {
    let bench = core.bench.lock().unwrap().clone();
    let ram_total = bench.as_ref().and_then(|b| b["ramBytes"].as_u64()).unwrap_or(0);
    let lang = core.lang();
    let m = &core.store.manifest;
    Value::Array(
        m.tiers
            .iter()
            .map(|t| {
                let ids = m.tier_models(t, &lang);
                let ram: u64 = ids.iter().filter_map(|i| m.model(i)).map(|x| x.ram).sum();
                let disk: u64 = ids.iter().filter_map(|i| m.model(i)).map(|x| x.size).sum();
                let llm_size = m.model(&t.llm).map(|x| x.size as f64 / 1e9).unwrap_or(1.0);
                let speed = bench.as_ref().map(|b| {
                    let tps = b["llmTokensPerS"].as_f64().map(|v| v * 1.28 / llm_size).or_else(|| b["bandwidth"].as_f64().map(|bw| bw * 0.55 / llm_size));
                    json!({ "asrRtf": b["asrRtf"], "llmTokensPerS": tps.map(|v| (v * 10.0).round() / 10.0) })
                });
                json!({
                    "id": t.id, "name": t.name,
                    "models": { "asr": t.asr, "llm": t.llm, "tts": t.tts },
                    "ramBytes": ram, "diskBytes": disk, "speed": speed, "reason": t.summary,
                    "fits": ram_total == 0 || ram_total >= t.min_ram,
                    "installed": ids.iter().all(|i| core.store.is_installed(i)),
                })
            })
            .collect(),
    )
}

pub fn models_list(core: &Core) -> Value {
    let bench = core.bench.lock().unwrap().clone();
    let total = crate::models::total_ram();
    let gpu = crate::gpu::best();
    let mut catalog = core.store.catalog_json();
    if let Some(a) = catalog.as_array_mut() {
        for m in a {
            let (fits, slow) = crate::models::fit(&core.store.manifest, m["id"].as_str().unwrap_or(""), total, gpu.as_ref());
            m["fits"] = json!(fits);
            m["slow"] = json!(slow);
        }
    }
    let installed: Vec<Value> = core
        .store
        .installed_list()
        .iter()
        .map(|i| {
            let def = core.store.manifest.model(&i.id);
            let in_use = core.slots().values().any(|s| s.models.contains(&i.id));
            json!({ "id": i.id, "slot": def.map(|d| d.slot.clone()), "name": def.map(|d| d.name.clone()).unwrap_or(i.id.clone()), "diskBytes": i.bytes, "verified": true, "verifiedAt": i.verified_at, "inUse": in_use })
        })
        .collect();
    json!({
        "slots": slots_json(core),
        "manifestVersion": core.store.manifest.version,
        "tiers": tiers_json(core),
        "tier": core.tier(),
        "recommended": bench.as_ref().and_then(|b| b["recommended"].as_str().map(str::to_owned)),
        "reason": bench.as_ref().and_then(|b| b["reason"].as_str().map(str::to_owned)),
        "catalog": catalog,
        "systemRamBytes": total,
        "installed": installed,
        "diskUsedBytes": core.store.disk_used(),
        "freeBytes": crate::models::free_bytes(&core.paths.models),
        "benchmarks": crate::bench::history(core),
        "downloads": core.downloads.list(),
        "gpu": crate::gpu::json(),
        "pending": core.pending_use.lock().unwrap().iter().map(|(id, slot, voice)| json!({ "id": id, "slot": slot, "voice": voice })).collect::<Vec<_>>(),
    })
}

fn history_filters(p: &Value) -> (String, Vec<rusqlite::types::Value>) {
    let mut w = vec!["1 = 1".to_owned()];
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    let f = p.get("filters").cloned().unwrap_or(json!({}));
    if let Some(from) = f.opt_i64("from") {
        w.push("time >= ?".into());
        args.push(from.into());
    }
    if let Some(to) = f.opt_i64("to") {
        w.push("time <= ?".into());
        args.push(to.into());
    }
    if let Some(r) = f.opt_str("result").filter(|r| !r.is_empty()) {
        w.push("result = ?".into());
        args.push(r.to_owned().into());
    }
    if let Some(a) = f.opt_str("app").filter(|a| !a.is_empty()) {
        w.push("app = ?".into());
        args.push(a.to_owned().into());
    }
    if let Some(q) = f.opt_str("text").and_then(fts_query) {
        w.push("id IN (SELECT rowid FROM history_fts WHERE history_fts MATCH ?)".into());
        args.push(q.into());
    }
    (w.join(" AND "), args)
}

fn history_query(core: &Core, p: &Value) -> ApiResult {
    let limit = p.opt_i64("limit").unwrap_or(200).clamp(1, 200);
    let offset = p.opt_str("cursor").and_then(|c| c.strip_prefix("o:")).and_then(|c| c.parse::<i64>().ok()).unwrap_or(0).max(0);
    let sort = p.get("sort").cloned().unwrap_or(json!({}));
    let col = match sort.opt_str("key").unwrap_or("time") {
        "duration" => "duration_ms",
        "result" => "result",
        "said" => "said",
        "app" => "app",
        _ => "time",
    };
    let dir = if sort.opt_bool("desc").unwrap_or(true) { "DESC" } else { "ASC" };
    let (w, args) = history_filters(p);
    core.db
        .read(|c| {
            let total: i64 = c.query_row(&format!("SELECT count(*) FROM history WHERE {w}"), params_from_iter(args.iter()), |r| r.get(0))?;
            let mut a2 = args.clone();
            a2.push(limit.into());
            a2.push(offset.into());
            let mut st = c.prepare(&format!("SELECT {HISTORY_COLS} FROM history WHERE {w} ORDER BY {col} {dir}, id {dir} LIMIT ? OFFSET ?"))?;
            let items: Vec<Value> = st.query_map(params_from_iter(a2.iter()), history_json)?.flatten().map(|v| core.live_undo(v)).collect();
            let next = (offset + (items.len() as i64) < total && items.len() as i64 == limit).then(|| format!("o:{}", offset + limit));
            Ok(json!({ "items": items, "nextCursor": next, "total": total }))
        })
        .map_err(internal)
}

fn csv_cell(s: &str) -> String {
    let mut t = s.replace('"', "\"\"");
    if t.starts_with(['=', '+', '-', '@']) {
        t.insert(0, '\'');
    }
    format!("\"{t}\"")
}

fn history_export(core: &Core, p: &Value) -> ApiResult {
    let path = p.str_field("path")?.to_owned();
    if !std::path::Path::new(&path).is_absolute() {
        return Err(ApiError::invalid("Choose a full path for the file."));
    }
    let format = p.opt_str("format").unwrap_or("csv").to_owned();
    let ids: Vec<i64> = p.str_list("ids").iter().filter_map(|s| s.parse().ok()).collect();
    let rows: Vec<Value> = core
        .db
        .read(|c| {
            let (sql, args): (String, Vec<rusqlite::types::Value>) = if ids.is_empty() {
                (format!("SELECT {HISTORY_COLS} FROM history ORDER BY time DESC"), vec![])
            } else {
                (format!("SELECT {HISTORY_COLS} FROM history WHERE id IN ({}) ORDER BY time DESC", vec!["?"; ids.len()].join(",")), ids.iter().map(|i| (*i).into()).collect())
            };
            let mut st = c.prepare(&sql)?;
            let v: Vec<Value> = st.query_map(params_from_iter(args.iter()), history_json)?.flatten().collect();
            Ok(v)
        })
        .map_err(internal)?;
    let count = rows.len();
    let body = if format == "json" {
        serde_json::to_string_pretty(&rows).map_err(internal)?
    } else {
        let mut s = String::from("time,you said,what it did,result,layer,duration ms,app\n");
        for r in &rows {
            let t = jiff::Timestamp::from_millisecond(r["time"].as_i64().unwrap_or(0)).map(|t| t.to_string()).unwrap_or_default();
            s.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                csv_cell(&t),
                csv_cell(r["said"].as_str().unwrap_or("")),
                csv_cell(r["did"].as_str().unwrap_or("")),
                csv_cell(r["result"].as_str().unwrap_or("")),
                csv_cell(r["layer"].as_str().unwrap_or("")),
                r["durationMs"],
                csv_cell(r["app"].as_str().unwrap_or(""))
            ));
        }
        s
    };
    std::fs::write(&path, body).map_err(|e| ApiError::new(ErrorCode::PermissionDenied, format!("The file could not be saved: {e}")))?;
    Ok(json!({ "path": path, "count": count }))
}

fn store_rows(core: &Core, store: &str, offset: i64, limit: i64) -> Result<(Vec<Value>, bool), ApiError> {
    let s = store.to_owned();
    core.db
        .read(move |c| {
            let (sql, map): (&str, fn(&rusqlite::Row) -> rusqlite::Result<Value>) = match s.as_str() {
                "preferences" => ("SELECT key, value, count, updated FROM preferences ORDER BY updated DESC LIMIT ?1 OFFSET ?2", |r| {
                    Ok(json!({ "key": r.get::<_, String>(0)?, "value": r.get::<_, String>(1)?, "count": r.get::<_, i64>(2)?, "updated": r.get::<_, i64>(3)? }))
                }),
                "corrections" => ("SELECT id, time, heard, meant FROM corrections ORDER BY time DESC LIMIT ?1 OFFSET ?2", |r| {
                    Ok(json!({ "id": r.get::<_, i64>(0)?, "time": r.get::<_, i64>(1)?, "heard": r.get::<_, String>(2)?, "meant": r.get::<_, String>(3)? }))
                }),
                "memories" => ("SELECT id, time, text FROM memories ORDER BY time DESC LIMIT ?1 OFFSET ?2", |r| {
                    Ok(json!({ "id": r.get::<_, i64>(0)?, "time": r.get::<_, i64>(1)?, "text": r.get::<_, String>(2)? }))
                }),
                "notes" => ("SELECT id, time, kind, text FROM (SELECT id, time, 'note' AS kind, text FROM notes UNION ALL SELECT id, time, 'to-do', text FROM todos UNION ALL SELECT id, time, 'reminder', text FROM reminders) ORDER BY time DESC LIMIT ?1 OFFSET ?2", |r| {
                    Ok(json!({ "id": r.get::<_, i64>(0)?, "time": r.get::<_, i64>(1)?, "kind": r.get::<_, String>(2)?, "text": r.get::<_, String>(3)? }))
                }),
                "chats" => ("SELECT m.id, m.chat, c.title, m.time, m.role, m.text FROM messages m JOIN chats c ON c.id = m.chat ORDER BY m.chat, m.id LIMIT ?1 OFFSET ?2", |r| {
                    Ok(json!({ "id": r.get::<_, i64>(0)?, "chat": r.get::<_, i64>(1)?, "title": r.get::<_, String>(2)?, "time": r.get::<_, i64>(3)?, "role": r.get::<_, String>(4)?, "text": r.get::<_, String>(5)? }))
                }),
                "apps" => ("SELECT id, name, source, aliases, opened, hidden FROM apps WHERE present = 1 ORDER BY name LIMIT ?1 OFFSET ?2", |r| {
                    Ok(json!({ "id": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)?, "source": r.get::<_, String>(2)?, "aliases": r.get::<_, String>(3)?, "opened": r.get::<_, i64>(4)?, "hidden": r.get::<_, i64>(5)? != 0 }))
                }),
                _ => ("SELECT id, time, said, did, result, layer, duration_ms, app, action, undoable FROM history ORDER BY time DESC LIMIT ?1 OFFSET ?2", history_json),
            };
            let mut st = c.prepare(sql)?;
            let v: Vec<Value> = st.query_map(rusqlite::params![limit + 1, offset], map)?.flatten().collect();
            Ok(v)
        })
        .map(|mut v| {
            let more = v.len() as i64 > limit;
            v.truncate(limit as usize);
            (v, more)
        })
        .map_err(internal)
}

fn data_summary(core: &Core) -> ApiResult {
    core.db
        .read(|c| {
            let q = |sql: &str| c.query_row(sql, [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).unwrap_or((0, 0));
            let h = q("SELECT count(*), coalesce(sum(length(said) + length(did) + coalesce(length(action), 0) + 48), 0) FROM history");
            let p = q("SELECT count(*), coalesce(sum(length(key) + length(value) + 24), 0) FROM preferences");
            let a = q("SELECT count(*), coalesce(sum(length(name) + length(path) + length(exec) + length(aliases) + 32), 0) FROM apps WHERE present = 1");
            let k = q("SELECT count(*), coalesce(sum(length(heard) + length(meant) + 24), 0) FROM corrections");
            let n = q("SELECT (SELECT count(*) FROM notes) + (SELECT count(*) FROM todos) + (SELECT count(*) FROM reminders), coalesce((SELECT sum(length(text) + 24) FROM notes), 0) + coalesce((SELECT sum(length(text) + 32) FROM todos), 0) + coalesce((SELECT sum(length(text) + 32) FROM reminders), 0)");
            let ch = q("SELECT (SELECT count(*) FROM chats), coalesce((SELECT sum(length(title) + 32) FROM chats), 0) + coalesce((SELECT sum(length(text) + 40) FROM messages), 0)");
            let me = q("SELECT count(*), coalesce(sum(length(text) + 32), 0) FROM memories");
            Ok(json!({
                "stores": [
                    { "id": "chats", "name": "Chats", "items": ch.0, "bytes": ch.1 },
                    { "id": "memories", "name": "Things you asked me to remember", "items": me.0, "bytes": me.1 },
                    { "id": "history", "name": "Command history", "items": h.0, "bytes": h.1 },
                    { "id": "preferences", "name": "Learned preferences", "items": p.0, "bytes": p.1 },
                    { "id": "apps", "name": "App index", "items": a.0, "bytes": a.1 },
                    { "id": "corrections", "name": "Correction examples", "items": k.0, "bytes": k.1 },
                    { "id": "notes", "name": "Notes, to-dos and reminders", "items": n.0, "bytes": n.1 },
                ],
                "dataDir": core.paths.data,
                "logsDir": core.paths.logs,
                "databaseBytes": std::fs::metadata(core.db.path()).map(|m| m.len()).unwrap_or(0),
            }))
        })
        .map_err(internal)
}

async fn wipe(core: &Arc<Core>, p: &Value) -> ApiResult {
    let store = p.opt_str("store").map(str::to_owned);
    match store.as_deref() {
        Some(s @ ("history" | "preferences" | "corrections" | "apps" | "notes" | "chats" | "memories")) => {
            let sql = match s {
                "history" => "DELETE FROM history",
                "memories" => "DELETE FROM memories",
                "chats" => "DELETE FROM messages; DELETE FROM chats",
                "preferences" => "DELETE FROM preferences",
                "corrections" => "DELETE FROM corrections",
                "notes" => "DELETE FROM notes; DELETE FROM todos; DELETE FROM reminders",
                _ => "UPDATE apps SET aliases = '[]', opened = 0, hidden = 0",
            };
            core.db
                .write_async(move |c| {
                    c.execute_batch(sql)?;
                    Ok(())
                })
                .await
                .map_err(internal)?;
            if s == "chats" {
                core.threads.focus(None);
                core.hub.emit("chat.changed", json!({ "deleted": null, "all": true }));
            }
            Ok(json!({}))
        }
        Some(_) => Err(ApiError::invalid("Unknown data store.")),
        None => {
            if p.opt_str("confirm") != Some("DELETE") {
                return Err(ApiError::invalid("Type DELETE to confirm."));
            }
            core.db
                .write_async(|c| {
                    c.execute_batch(
                        "DELETE FROM history; DELETE FROM preferences; DELETE FROM corrections; DELETE FROM missing_apps; DELETE FROM speaker; DELETE FROM benchmarks; DELETE FROM settings; DELETE FROM notes; DELETE FROM todos; DELETE FROM reminders; DELETE FROM messages; DELETE FROM chats; DELETE FROM memories; UPDATE apps SET aliases = '[]', opened = 0, hidden = 0; VACUUM;",
                    )?;
                    Ok(())
                })
                .await
                .map_err(internal)?;
            core.threads.focus(None);
            core.hub.emit("chat.changed", json!({ "deleted": null, "all": true }));
            let kept: Map<String, Value> = ["setup.completed", "setup.step", "models.tier"].iter().map(|k| ((*k).to_owned(), core.settings.get(k))).collect();
            let mut defaults = core.settings.reset_defaults();
            core.settings.apply(&kept);
            core.settings.set_internal("setup.version", json!(crate::settings::SETUP_VERSION));
            defaults.extend(kept.clone());
            let mut save = kept;
            save.insert("setup.version".into(), json!(crate::settings::SETUP_VERSION));
            core.db.save_settings(save);
            core.hub.emit("settings.changed", json!({ "changes": defaults.clone() }));
            core.tell(ControlMsg::Settings(defaults));
            Ok(json!({}))
        }
    }
}

pub async fn settings_set(core: &Arc<Core>, p: &Value) -> ApiResult {
    let patch: Map<String, Value> = if let Some(k) = p.opt_str("key") {
        let mut m = Map::new();
        m.insert(k.to_owned(), p.get("value").cloned().unwrap_or(Value::Null));
        m
    } else {
        p.get("patch").and_then(Value::as_object).cloned().ok_or_else(|| ApiError::bad("key and value, or patch, is required"))?
    };
    let clean = core.settings.prepare(&patch)?;
    let voice = clean.get("tts.voice").and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_owned);
    if let Some(v) = &voice {
        let m = core.store.manifest.model(v).filter(|m| m.slot == "tts").ok_or_else(|| ApiError::invalid("That isn't a voice."))?;
        if !core.store.is_installed(v) {
            return Err(ApiError::invalid(format!("{} is not downloaded yet.", m.name)));
        }
    }
    if let Some(v) = clean.get("startup.atLogin").and_then(Value::as_bool) {
        let paths = core.paths.clone();
        crate::layers::blocking(move || crate::autostart::set(&paths, v)).await.map_err(internal)?.map_err(|e| ApiError::new(ErrorCode::PermissionDenied, format!("Start at sign-in could not be changed: {e}")))?;
    }
    let changed = core.settings.apply(&clean);
    if !changed.is_empty() {
        core.db.save_settings(changed.clone());
        if let Some(done) = changed.get("setup.completed").and_then(Value::as_bool) {
            crate::provision::remember(&core.paths, done);
        }
        if let Some(v) = changed.get("plugins.developerMode").and_then(Value::as_bool) {
            core.plugins.set_developer(v);
            core.hub.emit("plugins.state", json!({ "id": "*", "status": "changed", "signature": "unsigned" }));
        }
        if changed.contains_key("history.retention") || changed.contains_key("history.keep") {
            retention(core);
        }
        if let Some(v) = voice.filter(|_| changed.contains_key("tts.voice")) {
            let mut slot = core.slot("tts");
            if slot.provider != "local" || slot.models.first() != Some(&v) {
                slot.provider = "local".into();
                slot.models = vec![v];
                core.set_slot("tts", Some(slot));
                core.engines.tts.stop();
                for s in slots_json(core) {
                    core.hub.emit("models.state", s);
                }
            }
        }
        if changed.keys().any(|k| k == "models.tier" || k == "ui.language") {
            for e in core.engines.all() {
                e.stop();
            }
            for s in slots_json(core) {
                core.hub.emit("models.state", s);
            }
        }
        core.hub.emit("settings.changed", json!({ "changes": changed.clone() }));
        core.tell(ControlMsg::Settings(changed));
    }
    Ok(json!({ "settings": clean, "restartRequired": [] }))
}

pub fn retention(core: &Core) {
    let cutoff = match core.settings.str("history.retention").as_str() {
        "30d" => Some(now_ms() - 30 * 86_400_000),
        "1y" => Some(now_ms() - 365 * 86_400_000),
        _ => None,
    };
    let keep = core.settings.bool("history.keep");
    core.db.write(move |c| {
        if !keep {
            c.execute("DELETE FROM history", [])?;
        } else if let Some(t) = cutoff {
            c.execute("DELETE FROM history WHERE time < ?1", [t])?;
        }
        Ok(())
    });
}

fn apps_list(core: &Core, p: &Value) -> ApiResult {
    let q = p.opt_str("query").map(crate::text::normalize).filter(|q| !q.is_empty());
    let include_hidden = p.opt_bool("includeHidden").unwrap_or(true);
    let items: Vec<Value> = core
        .apps
        .all()
        .iter()
        .filter(|a| include_hidden || !a.hidden)
        .filter(|a| q.as_ref().is_none_or(|q| crate::text::normalize(&a.name).contains(q.as_str()) || a.aliases.iter().any(|al| crate::text::normalize(al).contains(q.as_str()))))
        .map(|a| a.json())
        .collect();
    let defaults = core.defaults();
    let mut d = Map::new();
    let mut cands = Map::new();
    for r in crate::tools::apps::ROLES {
        d.insert((*r).into(), json!(core.apps.role_default(r, &defaults).map(|a| a.id)));
        cands.insert((*r).into(), json!(core.apps.candidates(r)));
    }
    let missing = core
        .db
        .read(|c| {
            let mut st = c.prepare("SELECT name, count, last_asked FROM missing_apps WHERE dismissed = 0 ORDER BY last_asked DESC LIMIT 50")?;
            let v: Vec<Value> = st.query_map([], |r| Ok(json!({ "name": r.get::<_, String>(0)?, "count": r.get::<_, i64>(1)?, "lastAsked": r.get::<_, i64>(2)? })))?.flatten().collect();
            Ok(v)
        })
        .map_err(internal)?;
    Ok(json!({
        "items": items, "missing": missing, "defaults": d, "candidates": cands,
        "scanning": core.apps.scanning.load(std::sync::atomic::Ordering::Relaxed), "count": core.apps.count()
    }))
}

pub async fn rescan(core: Arc<Core>) -> ApiResult {
    core.hub.emit("apps.changed", json!({ "count": core.apps.count(), "delta": 0, "scanning": true }));
    let c2 = core.clone();
    let before: Vec<String> = core.apps.all().into_iter().map(|a| a.name).collect();
    let r = crate::layers::blocking(move || c2.apps.scan()).await.map_err(internal)?.map_err(internal)?;
    let after: Vec<String> = core.apps.all().into_iter().map(|a| a.name).collect();
    if before != after {
        crate::pipeline::reset_prefix();
    }
    core.hub.emit("apps.changed", json!({ "count": r.0, "delta": r.1, "scanning": false }));
    Ok(json!({ "count": r.0 }))
}

fn confirm(core: &Core, spec: ConfirmSpec, then: Box<dyn FnOnce(Arc<Core>) + Send>) {
    core.tell(ControlMsg::Req(Req::Confirm { spec, then }));
}

pub async fn dispatch(core: &Arc<Core>, method: &str, p: Value) -> ApiResult {
    match method {
        "health.get" => {
            let max = core.control.max_capacity();
            core.metrics.queue("control", max - core.control.capacity(), max);
            let (d, c) = core.engines.asr.queue_depth();
            core.metrics.queue("asr_audio", d, c);
            Ok(json!({ "daemon": core.daemon_state(), "latency": core.metrics.latency(), "layers": crate::layers::latency(), "queues": core.metrics.queues(), "memory": { "rssBytes": crate::metrics::rss_bytes() }, "work": crate::work::state(core.downloads.worker_running()) }))
        }
        "health.layers" => Ok(json!({ "latency": crate::layers::latency(), "samples": crate::layers::samples() })),
        "daemon.quit" => {
            core.tell(ControlMsg::Shutdown);
            crate::shutdown();
            Ok(json!({}))
        }
        "session.sleep" => control::ask(core, |r| Req::Sleep { reply: r }).await,
        "session.attend" => {
            tokio::spawn(crate::tools::warm());
            Ok(json!({}))
        }
        "session.listen" => {
            let chat = p.get("chat").and_then(Value::as_i64);
            control::ask(core, |r| Req::Listen { chat, reply: r }).await
        }
        "notch.state" => Ok(crate::board::state(core)),
        "notch.action" => crate::board::action(core, &p).await,
        "sound.play" => {
            let cue = crate::sounds::Cue::parse(p.str_field("name")?).ok_or_else(|| ApiError::invalid("No such sound."))?;
            crate::sounds::play(core, cue);
            Ok(json!({}))
        }
        "attention.dismiss" => {
            core.attention.clear(&core.hub, p.str_field("code")?);
            Ok(json!({}))
        }
        "settings.get" => {
            let all = core.settings.all_public();
            let keys = p.str_list("keys");
            let out: Map<String, Value> = if keys.is_empty() { all } else { all.into_iter().filter(|(k, _)| keys.contains(k)).collect() };
            Ok(json!({ "settings": out }))
        }
        "settings.set" => settings_set(core, &p).await,
        "mic.setMuted" => {
            let muted = p.bool_field("muted")?;
            let minutes = p.opt_f64("minutes").filter(|m| *m > 0.0 && *m <= 1440.0);
            control::ask(core, |r| Req::Mute { muted, minutes, reply: r }).await
        }
        "audio.devices" => {
            let d = crate::layers::blocking(setu_audio::devices).await.map_err(internal)?;
            let map = |v: Vec<setu_audio::Device>| v.into_iter().map(|d| json!({ "id": d.id, "name": d.name, "isDefault": d.is_default })).collect::<Vec<_>>();
            Ok(json!({ "inputs": map(d.inputs), "outputs": map(d.outputs) }))
        }
        "wake.checkName" => Ok(crate::names::check(p.str_field("name")?, p.opt_bool("useHey").unwrap_or(true))),
        "wake.test.start" => control::ask(core, |r| Req::WakeTest { start: true, reply: r }).await,
        "wake.test.stop" => control::ask(core, |r| Req::WakeTest { start: false, reply: r }).await,
        "voice.sample" => {
            let voice = p.opt_str("voice").map(str::to_owned);
            let (rate, volume) = (p.opt_f64("rate"), p.opt_f64("volume"));
            let text = p.opt_str("text").map(|t| t.chars().take(400).collect::<String>());
            control::ask(core, |r| Req::Sample { voice, rate, volume, text, reply: r }).await
        }
        "onboarding.listen" => {
            let say = p.opt_str("say").map(|t| t.chars().take(600).collect::<String>());
            let choices: Option<Vec<(String, Vec<String>)>> = p.get("choices").and_then(Value::as_array).map(|a| {
                a.iter()
                    .take(16)
                    .filter_map(|c| {
                        let id: String = c.get("id")?.as_str()?.chars().take(64).collect();
                        let phrases: Vec<String> = c.get("phrases")?.as_array()?.iter().filter_map(Value::as_str).map(crate::text::spoken_phrase).filter(|x| !x.is_empty()).take(6).collect();
                        (!id.is_empty() && !phrases.is_empty()).then_some((id, phrases))
                    })
                    .collect()
            });
            control::ask(core, |r| Req::Onboard { say, choices, reply: r }).await
        }
        "speaker.enroll" => {
            let text = p.opt_str("text").map(|t| t.chars().take(600).collect::<String>()).filter(|t| !t.trim().is_empty());
            control::ask(core, |r| Req::Enroll { text, reply: r }).await
        }
        "onboarding.converse" => {
            let screen = p.get("screen").filter(|v| v.is_object()).cloned();
            let explain = p.opt_bool("explain").unwrap_or(false);
            control::ask(core, |r| Req::Converse { screen, explain, reply: r }).await
        }
        "onboarding.text" => {
            let text: String = p.str_field("text")?.chars().take(300).collect();
            control::ask(core, |r| Req::ConverseText { text, reply: r }).await
        }
        "onboarding.paragraph" => Ok(json!({ "text": paragraph(core).await })),
        "prompt.answer" => {
            let id = p.str_field("id")?.to_owned();
            let answer = p.get("answer").cloned().ok_or_else(|| ApiError::bad("answer is required"))?;
            let via = p.opt_str("via").unwrap_or("click").to_owned();
            control::ask(core, |r| Req::Answer { id, answer, via, reply: r }).await
        }
        "action.cancel" => {
            let id = p.opt_str("id").map(str::to_owned);
            control::ask(core, |r| Req::Cancel { id, reply: r }).await
        }
        "chat.list" => crate::threads::list(core, p.get("limit").and_then(Value::as_i64).unwrap_or(200).clamp(1, 500), p.opt_str("query").unwrap_or("")).map_err(internal),
        "chat.get" => {
            let id = p.get("id").and_then(Value::as_i64).ok_or_else(|| ApiError::bad("id is required"))?;
            crate::threads::get(core, id).map_err(internal)
        }
        "chat.focus" => {
            core.threads.focus(p.get("id").and_then(Value::as_i64));
            Ok(json!({}))
        }
        "chat.rename" => {
            let id = p.get("id").and_then(Value::as_i64).ok_or_else(|| ApiError::bad("id is required"))?;
            crate::threads::rename(core, id, p.str_field("title")?).map_err(internal)
        }
        "chat.delete" => {
            let id = p.get("id").and_then(Value::as_i64);
            if id.is_none() && !p.get("all").and_then(Value::as_bool).unwrap_or(false) {
                return Err(ApiError::bad("id or all is required"));
            }
            crate::threads::delete(core, id).map_err(internal)?;
            Ok(json!({}))
        }
        "chat.send" => {
            let text = p.str_field("text")?.trim().to_owned();
            if text.is_empty() {
                return Err(ApiError::invalid("Type a message."));
            }
            if text.chars().count() > 4000 {
                return Err(ApiError::invalid("That message is too long."));
            }
            let chat = match p.get("chat").and_then(Value::as_i64).filter(|c| crate::threads::exists(core, *c)) {
                Some(c) => c,
                None => crate::threads::create(core, &crate::threads::title_from(&text)).map_err(internal)?,
            };
            crate::threads::add(core, chat, "user", &text, None).map_err(internal)?;
            core.threads.focus(Some(chat));
            match control::ask(core, |r| Req::RunChat { text, chat, reply: r }).await {
                Ok(v) => Ok(json!({ "chat": chat, "interaction": v.get("interaction").cloned().unwrap_or(Value::Null) })),
                Err(e) => {
                    let _ = crate::threads::add(core, chat, "assistant", &e.message, None);
                    Ok(json!({ "chat": chat, "interaction": null, "error": e.message }))
                }
            }
        }
        "debug.heard" if std::env::var_os("SETU_BENCH").is_some() => {
            if p.get("forget").and_then(Value::as_bool) == Some(true) {
                crate::speaker::forget_recent();
                return Ok(json!({}));
            }
            let text = p.str_field("text")?.to_owned();
            let score = p.get("score").and_then(Value::as_f64).map(|s| s as f32);
            let seconds = p.get("seconds").and_then(Value::as_f64).unwrap_or(3.0) as f32;
            let addressed = p.get("addressed").and_then(Value::as_bool).unwrap_or(false);
            control::ask(core, |r| Req::Heard { text, score, seconds, addressed, reply: r }).await
        }
        "intent.run" => {
            let text = p.str_field("text")?.trim().to_owned();
            if text.chars().count() > 200 {
                return Err(ApiError::invalid("That command is too long."));
            }
            control::ask(core, |r| Req::RunText { text, reply: r }).await
        }
        "action.undo" => {
            let id = p.str_field("id")?.to_owned();
            control::ask(core, |r| Req::Undo { id, reply: r }).await
        }
        "models.list" => Ok(models_list(core)),
        "models.set" => {
            let slot = p.str_field("slot")?;
            if !["asr", "llm", "tts"].contains(&slot) {
                return Err(ApiError::invalid("slot must be asr, llm or tts"));
            }
            let provider = p.str_field("provider")?;
            if provider != "local" && !crate::remote::providers(slot).contains(&provider) {
                return Err(ApiError::invalid("Unknown provider."));
            }
            let mut cfg = SlotConfig { provider: provider.into(), ..Default::default() };
            if provider == "local" {
                if let Some(path) = p.opt_str("path").filter(|s| !s.is_empty()) {
                    let pp = std::path::Path::new(path);
                    let ext = pp.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                    match slot {
                        "llm" if pp.is_absolute() && pp.is_file() && ext == "gguf" => {}
                        "llm" => return Err(ApiError::invalid("Choose a .gguf language model file.")),
                        "tts" if pp.is_absolute() && pp.is_file() && ext == "onnx" && pp.with_file_name("tokens.txt").is_file() => {}
                        "tts" => return Err(ApiError::invalid("Choose a Piper voice .onnx file that has tokens.txt in the same folder. Voices from the sherpa-onnx text-to-speech models page work.")),
                        _ => return Err(ApiError::invalid("A file can't be used for speech recognition.")),
                    }
                    if slot == "llm" {
                        let size = std::fs::metadata(pp).map(|m| m.len()).unwrap_or(0);
                        let total = crate::models::total_ram();
                        if total > 0 && size + 1_500_000_000 > total {
                            return Err(ApiError::invalid("Oops, this model is too much for this computer. You can use an online model instead."));
                        }
                    }
                    cfg.path = Some(path.into());
                    cfg.voice = p.opt_str("voice").filter(|v| v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).map(str::to_owned);
                } else if let Some(m) = p.opt_str("model") {
                    cfg = local_slot(core, slot, m, p.opt_str("voice").map(str::to_owned))?;
                } else {
                    core.set_slot(slot, None);
                    let s = slot_state(core, slot, None, None, 0);
                    core.hub.emit("models.state", s.clone());
                    return Ok(s);
                }
            } else {
                cfg.base_url = p.opt_str("baseUrl").filter(|u| !u.is_empty()).map(str::to_owned);
                cfg.model_name = p.opt_str("modelName").map(str::to_owned);
                cfg.key_ref = p.opt_str("keyRef").filter(|r| !r.is_empty()).map(str::to_owned);
                cfg.voice = p.opt_str("voice").filter(|v| !v.is_empty()).map(|v| v.chars().take(64).collect());
                if cfg.key_ref.as_deref().is_some_and(|r| !crate::secrets::valid_ref(r)) {
                    return Err(ApiError::invalid("Invalid key reference."));
                }
                crate::remote::base(&cfg).map_err(|e| ApiError::invalid(e.to_string()))?;
                if cfg.model_name.as_deref().is_none_or(str::is_empty) && !(slot == "tts" && provider == "elevenlabs") {
                    return Err(ApiError::invalid("Enter the model name."));
                }
                if slot == "tts" && provider == "elevenlabs" && cfg.voice.is_none() {
                    return Err(ApiError::invalid("Choose a voice."));
                }
            }
            core.pending_use.lock().unwrap().retain(|(_, s, _)| s != slot);
            Ok(apply_slot(core, slot, Some(cfg)))
        }
        "models.use" => use_model(core, &p),
        "models.warm" => {
            crate::pipeline::ensure_asr(core).await.map_err(|e| ApiError::unavailable(format!("Hearing could not start: {e}")))?;
            crate::pipeline::ensure_llm(core).await.map_err(|e| ApiError::unavailable(format!("The brain could not start: {e}")))?;
            crate::pipeline::ensure_tts(core).await.map_err(|e| ApiError::unavailable(format!("The voice could not start: {e}")))?;
            let c = core.clone();
            tokio::spawn(async move { crate::pipeline::warm(&c).await });
            Ok(json!({
                "asr": core.engines.asr.device(),
                "llm": core.engines.llm.device(),
                "tts": core.engines.tts.device(),
                "gpu": crate::gpu::json(),
            }))
        }
        "models.testConnection" => {
            let cfg = SlotConfig {
                provider: p.opt_str("provider").unwrap_or("openai").into(),
                base_url: p.opt_str("baseUrl").filter(|u| !u.is_empty()).map(str::to_owned),
                model_name: p.opt_str("modelName").map(str::to_owned),
                key_ref: p.opt_str("keyRef").filter(|r| !r.is_empty()).map(str::to_owned),
                ..Default::default()
            };
            Ok(crate::remote::test(p.opt_str("slot").unwrap_or("llm"), &cfg).await)
        }
        "models.download" => {
            let mut ids = p.str_list("ids");
            if let Some(t) = p.opt_str("tier") {
                let tier = core.store.manifest.tier(t).ok_or_else(|| ApiError::not_found("No such tier."))?;
                ids.extend(core.store.manifest.tier_models(tier, &core.lang()));
            }
            if ids.is_empty() {
                return Err(ApiError::bad("ids or tier is required"));
            }
            let ids = core.store.manifest.with_requires(&ids);
            core.downloads.enqueue(&ids)
        }
        "models.pause" => core.downloads.pause(p.str_field("id")?).map(|_| json!({})),
        "models.resume" => core.downloads.resume(p.str_field("id")?).map(|_| json!({})),
        "models.cancel" => core.downloads.cancel(p.str_field("id")?).map(|_| json!({})),
        "models.delete" => {
            let id = p.str_field("id")?;
            if core.slots().values().any(|s| s.models.iter().any(|m| m == id)) {
                for e in core.engines.all() {
                    e.stop();
                }
            }
            let freed = core.store.delete(id).map_err(|e| ApiError::invalid(e.to_string()))?;
            for s in slots_json(core) {
                core.hub.emit("models.state", s);
            }
            Ok(json!({ "freedBytes": freed }))
        }
        "benchmark.run" => {
            if core.bench_running.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(ApiError::new(ErrorCode::Busy, "A system scan is already running."));
            }
            tokio::spawn(crate::bench::run(core.clone()));
            Ok(json!({}))
        }
        "benchmark.cancel" => {
            core.bench_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(json!({}))
        }
        "permissions.status" => Ok(crate::permissions::status(core).await),
        "permissions.request" => {
            let item = p.str_field("item")?.to_owned();
            let mut patch = Map::new();
            let mut opened = false;
            match item.as_str() {
                "accessibility" => {
                    let _ = crate::tools::atspi::set_enabled(true).await;
                    patch.insert("permissions.control".into(), json!(true));
                }
                "screen" => {
                    patch.insert("privacy.screenReading".into(), json!(true));
                }
                "microphone" | "camera" | "automation" => opened = crate::permissions::open_settings(&item),
                "folders" => {
                    patch.insert("permissions.files".into(), json!(true));
                }
                _ => return Err(ApiError::not_found("Unknown permission.")),
            }
            if !patch.is_empty() {
                settings_set(core, &json!({ "patch": patch })).await?;
            }
            let st = crate::permissions::status(core).await;
            let status = st["tiers"].as_array().into_iter().flatten().flat_map(|t| t["items"].as_array().cloned().unwrap_or_default()).find(|i| i["id"] == item.as_str()).map(|i| i["status"].clone()).unwrap_or(json!("unknown"));
            core.hub.emit("permissions.state", json!({ "tier": "", "item": item, "status": status }));
            Ok(json!({ "status": status, "opened": opened }))
        }
        "permissions.reset" => {
            let patch = json!({ "patch": { "permissions.files": false, "permissions.control": false, "privacy.screenReading": false, "files.folders": [] } });
            settings_set(core, &patch).await?;
            Ok(json!({}))
        }
        "media.now" => {
            core.nowplaying.refresh();
            crate::nowplaying::refresh_rows().await;
            Ok(core.nowplaying.json())
        }
        "media.control" => {
            let key = p.str_field("key")?.to_owned();
            let action = p.str_field("action")?.to_owned();
            crate::nowplaying::control(core, &key, &action).await.map_err(ApiError::invalid)
        }
        "browsers.list" => Ok(crate::tools::browsers::list(core)),
        "browsers.choose" => {
            let id = p.str_field("id")?.to_owned();
            crate::browse::choose_and_connect(core.clone(), id).await.map_err(ApiError::invalid)
        }
        "browsers.setup" => {
            let id = p.opt_str("id").map(str::to_owned).or_else(|| crate::tools::browsers::chosen(core)).ok_or_else(|| ApiError::invalid("Choose a browser first."))?;
            crate::browse::choose_and_connect(core.clone(), id).await.map_err(ApiError::invalid)
        }
        "apps.list" => apps_list(core, &p),
        "look.state" => Ok(crate::looks::state()),
        "vault.submit" => crate::vault::submit(core, &p).await.map_err(ApiError::invalid),
        "vault.cancel" => Ok(crate::vault::cancel(core, p.str_field("id")?)),
        "vault.hide" => Ok(crate::vault::hide(core)),
        "vault.entry" => Ok(crate::vault::current()),
        "face.cancel" => Ok(crate::faceid::cancel()),
        "face.state" => Ok(crate::faceid::state(core).await),
        "jobs.state" => Ok(crate::jobs::state()),
        "look.act" => {
            let op = p.str_field("op")?.to_owned();
            if matches!(op.as_str(), "create" | "import" | "cancel" | "save" | "confirm" | "back" | "rename" | "edit") {
                let intent = crate::router::Intent::Act { tool: "look.step".into(), args: p.clone() };
                return control::ask(core, |r| Req::RunIntent { intent, reply: r }).await;
            }
            let value = p.opt_str("value").unwrap_or_default().to_owned();
            crate::looks::change(core, &op, &value).map(|s| crate::looks::state_of(&s)).map_err(ApiError::invalid)
        }
        "browser.launch" => {
            let key = p.str_field("key")?.to_owned();
            let list = |k: &str| p.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>()).unwrap_or_default();
            crate::browse::launch_entry(core, &key, list("urls"), list("flags")).await.map_err(ApiError::invalid)
        }
        "apps.rescan" => rescan(core.clone()).await,
        "apps.setAlias" => core.apps.set_aliases(p.str_field("id")?, p.str_list("aliases")).map(|a| a.json()).map_err(|e| ApiError::not_found(e.to_string())),
        "apps.setDefault" => {
            let role = p.str_field("role")?;
            if !crate::tools::apps::ROLES.contains(&role) {
                return Err(ApiError::invalid("Unknown role."));
            }
            let mut d = core.settings.get("apps.defaults").as_object().cloned().unwrap_or_default();
            match p.opt_str("id") {
                Some(id) => {
                    core.apps.get(id).ok_or_else(|| ApiError::not_found("No such app."))?;
                    d.insert(role.into(), json!(id));
                }
                None => {
                    d.remove(role);
                }
            }
            settings_set(core, &json!({ "key": "apps.defaults", "value": d })).await?;
            crate::pipeline::reset_prefix();
            Ok(json!({}))
        }
        "apps.hide" => {
            let a = core.apps.set_hidden(p.str_field("id")?, p.bool_field("hidden")?).map_err(|e| ApiError::not_found(e.to_string()))?;
            crate::pipeline::reset_prefix();
            core.hub.emit("apps.changed", json!({ "count": core.apps.count(), "delta": 0, "scanning": false }));
            Ok(a.json())
        }
        "apps.add" => {
            let a = core.apps.add_manual(p.str_field("path")?).map_err(|e| ApiError::invalid(e.to_string()))?;
            crate::pipeline::reset_prefix();
            core.hub.emit("apps.changed", json!({ "count": core.apps.count(), "delta": 1, "scanning": false }));
            Ok(a.json())
        }
        "apps.install" => {
            let name = p.str_field("name")?.to_owned();
            let c = core.clone();
            let pkg = crate::layers::blocking(move || crate::tools::install::search(&name)).await.map_err(internal)?.map_err(ApiError::not_found)?;
            let spec = crate::tools::install::confirm_spec(&pkg);
            confirm(
                core,
                spec,
                Box::new(move |core| {
                    tokio::spawn(async move {
                        let flag = std::sync::atomic::AtomicBool::new(false);
                        let p2 = pkg.clone();
                        let r = crate::layers::blocking(move || crate::tools::install::install(&p2, &flag)).await;
                        match r {
                            Ok(Ok(())) => {
                                let _ = rescan(core.clone()).await;
                                let n = pkg.name.clone();
                                core.db.write(move |c| {
                                    c.execute("UPDATE missing_apps SET dismissed = 1 WHERE name = ?1 COLLATE NOCASE", [n])?;
                                    Ok(())
                                });
                            }
                            Ok(Err(e)) => core.hub.emit("notice.problem", json!({ "interaction": null, "message": e, "action": null })),
                            Err(e) => log::error!("install task failed: {e}"),
                        }
                    });
                }),
            );
            let _ = c;
            Ok(json!({}))
        }
        "apps.dismissMissing" => {
            let name = p.str_field("name")?.to_owned();
            core.db
                .write_async(move |c| {
                    c.execute("UPDATE missing_apps SET dismissed = 1 WHERE name = ?1", [name])?;
                    Ok(())
                })
                .await
                .map_err(internal)?;
            core.hub.emit("apps.changed", json!({ "count": core.apps.count(), "delta": 0, "scanning": false }));
            Ok(json!({}))
        }
        "history.query" => history_query(core, &p),
        "history.correct" => {
            let id: i64 = p.str_field("id")?.parse().map_err(|_| ApiError::bad("id"))?;
            if p.opt_bool("listen").unwrap_or(false) {
                return control::ask(core, |r| Req::CorrectListen { history: id, reply: r }).await;
            }
            let text = p.str_field("text")?.trim().to_owned();
            if text.is_empty() || text.len() > 500 {
                return Err(ApiError::invalid("Type what you meant."));
            }
            if !core.settings.bool("privacy.learn") {
                return Err(ApiError::invalid("Learning from your commands is turned off in Privacy and data."));
            }
            core.db
                .write_async(move |c| {
                    let heard: String = c.query_row("SELECT said FROM history WHERE id = ?1", [id], |r| r.get(0))?;
                    c.execute("INSERT INTO corrections(time, heard, meant, history_id) VALUES(?1, ?2, ?3, ?4)", rusqlite::params![now_ms(), heard, text, id])?;
                    Ok(())
                })
                .await
                .map_err(|_| ApiError::not_found("That history item no longer exists."))?;
            Ok(json!({}))
        }
        "history.export" => history_export(core, &p),
        "history.delete" => {
            let all = p.opt_bool("all").unwrap_or(false);
            let ids: Vec<i64> = p.str_list("ids").iter().filter_map(|s| s.parse().ok()).collect();
            if !all && ids.is_empty() {
                return Err(ApiError::bad("ids or all is required"));
            }
            let n = core
                .db
                .write_async(move |c| {
                    if all {
                        Ok(c.execute("DELETE FROM history", [])?)
                    } else {
                        let sql = format!("DELETE FROM history WHERE id IN ({})", vec!["?"; ids.len()].join(","));
                        Ok(c.execute(&sql, params_from_iter(ids.iter()))?)
                    }
                })
                .await
                .map_err(internal)?;
            Ok(json!({ "deleted": n }))
        }
        "data.summary" => data_summary(core),
        "data.query" => {
            let store = p.str_field("store")?;
            if !["chats", "memories", "notes", "history", "preferences", "apps", "corrections"].contains(&store) {
                return Err(ApiError::invalid("Unknown data store."));
            }
            let offset = p.opt_str("cursor").and_then(|c| c.parse::<i64>().ok()).unwrap_or(0);
            let limit = p.opt_i64("limit").unwrap_or(100).clamp(1, 200);
            let (items, more) = store_rows(core, store, offset, limit)?;
            Ok(json!({ "items": items, "nextCursor": more.then(|| (offset + limit).to_string()) }))
        }
        "data.export" => {
            let path = p.str_field("path")?.to_owned();
            if !std::path::Path::new(&path).is_absolute() {
                return Err(ApiError::invalid("Choose a full path for the file."));
            }
            let stores: Vec<&str> = match p.opt_str("store") {
                Some(s) => vec![s],
                None => vec!["chats", "memories", "notes", "history", "preferences", "apps", "corrections"],
            };
            let mut out = Map::new();
            for s in stores {
                let (items, _) = store_rows(core, s, 0, 1_000_000)?;
                out.insert(s.into(), Value::Array(items));
            }
            let mut settings = core.settings.all_public();
            settings.remove("models.slots");
            out.insert("settings".into(), Value::Object(settings));
            std::fs::write(&path, serde_json::to_string_pretty(&out).map_err(internal)?).map_err(|e| ApiError::new(ErrorCode::PermissionDenied, e.to_string()))?;
            Ok(json!({ "path": path }))
        }
        "data.wipe" => wipe(core, &p).await,
        "plugins.list" => Ok(core.plugins.list()),
        "plugins.catalog" => Ok(json!({ "items": [] })),
        "plugins.install" => Err(ApiError::not_found("That plugin isn't in the catalog.")),
        "plugins.setEnabled" => {
            let id = p.str_field("id")?;
            let r = core.plugins.set_enabled(id, p.bool_field("enabled")?).map_err(ApiError::invalid)?;
            crate::pipeline::reset_prefix();
            core.hub.emit("plugins.state", json!({ "id": id, "status": if r["enabled"].as_bool() == Some(true) { "enabled" } else { "disabled" }, "signature": r["signature"] }));
            Ok(r)
        }
        "plugins.remove" => {
            let id = p.str_field("id")?.to_owned();
            let name = core.plugins.name(&id).ok_or_else(|| ApiError::not_found("No such plugin."))?;
            let spec = ConfirmSpec {
                title: format!("Remove {name}"),
                facts: vec![format!("plugin  {id}")],
                consequence: "The plugin and its files are deleted. Commands it added stop working.".into(),
                risk: "medium",
                action_label: format!("Remove {name}"),
                danger: true,
            };
            confirm(
                core,
                spec,
                Box::new(move |core| {
                    if core.plugins.remove(&id).is_ok() {
                        crate::pipeline::reset_prefix();
                        core.hub.emit("plugins.state", json!({ "id": id, "status": "removed", "signature": "unsigned" }));
                    }
                }),
            );
            Ok(json!({}))
        }
        "plugins.checkUpdate" => Ok(json!({ "available": false, "version": null })),
        "skills.state" => {
            let c = core.clone();
            crate::layers::blocking(move || crate::skills::state(&c)).await.map_err(|e| ApiError::internal(e.to_string()))
        }
        "files.open" => {
            let path = std::path::PathBuf::from(p.str_field("path")?);
            if !path.exists() {
                return Err(ApiError::not_found("That place isn't available any more."));
            }
            crate::layers::blocking(move || crate::tools::shell::open(&path.to_string_lossy())).await.map_err(|e| ApiError::internal(e.to_string()))?.map_err(|e| ApiError::invalid(e.to_string()))?;
            Ok(json!({}))
        }
        "numbers.show" => control::ask(core, |r| Req::Numbers { show: true, reply: r }).await,
        "numbers.hide" => control::ask(core, |r| Req::Numbers { show: false, reply: r }).await,
        "wifi.scan" => control::ask(core, |r| Req::WifiScan { reply: r }).await,
        "wifi.connect" => {
            let ssid = p.str_field("ssid")?.to_owned();
            control::ask(core, |r| Req::WifiConnect { ssid, reply: r }).await
        }
        "wifi.forget" => {
            let ssid = p.str_field("ssid")?.to_owned();
            let spec = ConfirmSpec {
                title: format!("Forget {ssid}"),
                facts: vec![format!("network  {ssid}")],
                consequence: "The saved password is deleted. You will need it again to reconnect.".into(),
                risk: "medium",
                action_label: format!("Forget {ssid}"),
                danger: true,
            };
            confirm(
                core,
                spec,
                Box::new(move |_core| {
                    tokio::spawn(async move {
                        if let Err(e) = crate::tools::wifi::forget(&ssid).await {
                            log::warn!("forget network failed: {e}");
                        }
                    });
                }),
            );
            Ok(json!({}))
        }
        "wifi.reveal" => control::ask(core, |r| Req::WifiReveal { reply: r }).await,
        "camera.open" => control::ask(core, |r| Req::CameraOpen { reply: r }).await,
        "camera.capture" => control::ask(core, |r| Req::CameraCapture { reply: r }).await,
        "camera.close" => control::ask(core, |r| Req::CameraClose { reply: r }).await,
        "diagnostics.preview" => Ok(crate::diagnostics::preview(core)),
        "diagnostics.export" => crate::diagnostics::export(core, p.str_field("path")?).map_err(|e| ApiError::invalid(e.to_string())),
        "update.check" => {
            let s = crate::update::check(core).await;
            core.hub.emit("update.state", s.clone());
            Ok(s)
        }
        "update.apply" => Err(ApiError::new(ErrorCode::Unsupported, "Updates for this build are installed through the system package manager or the download page.")),
        _ => Err(ApiError::new(ErrorCode::UnknownMethod, format!("{method} is not supported"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_cells_neutralize_formulas() {
        assert_eq!(csv_cell("=SUM(A1)"), "\"'=SUM(A1)\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn filters_build_parameterized_sql() {
        let (w, a) = history_filters(&json!({ "filters": { "result": "failed", "text": "fire fox", "from": 5 } }));
        assert!(w.contains("result = ?") && w.contains("MATCH ?") && w.contains("time >= ?"));
        assert_eq!(a.len(), 3);
    }
}
