//! The chain of catalogs pinned in the group: what was delivered, each catalog naming the one before it.
use ks_cast::To as _;
use std::collections::HashSet;

use serde_json::{Value, json};

use crate::config::Config;
use crate::constants::{CATALOG_MAX_ENTRIES, MAX_DOWNLOAD, CATALOG_MAX_VERSION_ENTRIES, INDEX_LIMIT, DELIVERY_BATCH_LIMIT};
use crate::error::{Error, Result};
use crate::rate_limit::Cancelled;
use crate::store::{HEAD_DIGEST_KEY, LEGACY_HEAD_KEY, PENDING_KEY, Store, dumps};
use crate::telegram::{Client, is_file_id};
use crate::timeutil::{iso_date, iso_now, new_id, sha256_hex};
use crate::versions::{MARKER_KIND, caption, current_versions, is_version};

pub const CATALOG_CAPTION: &str = "LogCourier catalog SHA256 ";
const LOST_PIN: &str = "Закрепление каталога потеряно или изменено. Верните последнее закрепление LogCourier или начните каталог заново: «Отправка» → «Очистить группу и начать заново».";

fn lower_hex(text: &str, length: usize) -> bool {
    text.len() == length && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn checkpoint(cancelled: &Cancelled) -> Result<()> {
    if cancelled() {
        return Err(Error::Cancelled("Отправка остановлена. Очередь сохранена.".to_owned()));
    }
    Ok(())
}

fn valid_index(result: &Value, chat_id: &str, bot_id: &str) -> Option<()> {
    fn id32(value: &Value) -> Option<&str> {
        value.as_str().filter(|text| lower_hex(text, 32))
    }
    if result["schema"].as_i64()? != 1 || result["kind"].as_str()? != "logcourier.index" {
        return None;
    }
    if result["chat_id"].as_str()? != chat_id || result["bot_id"].as_str()? != bot_id {
        return None;
    }
    id32(&result["device_id"])?;
    let entries = result["entries"].as_array()?;
    if entries.len() > CATALOG_MAX_ENTRIES {
        return None;
    }
    for entry in entries {
        id32(&entry["bundle_id"])?;
        if !is_file_id(entry["file_id"].as_str()?) || !lower_hex(entry["sha256"].as_str()?, 64) {
            return None;
        }
        let size = entry["size"].as_i64()?;
        if size <= 0 || size.to::<usize>() > MAX_DOWNLOAD {
            return None;
        }
        iso_date(entry["created_at"].as_str()?)?;
        let version = &entry["keyswitch_version"];
        if !version.is_null() {
            if !is_version(version.as_str()?) {
                return None;
            }
            id32(&entry["source_id"])?;
            if entry["device_id"] != result["device_id"] {
                return None;
            }
        }
        if entry["kind"].as_str() == Some(MARKER_KIND) && version.is_null() {
            return None;
        }
    }
    let empty = json!({});
    let versions = result.get("keyswitch_versions").unwrap_or(&empty).as_object()?;
    if versions.len() > CATALOG_MAX_VERSION_ENTRIES {
        return None;
    }
    for (source, state) in versions {
        if !lower_hex(source, 32) || !is_version(state["version"].as_str()?) {
            return None;
        }
        id32(&state["marker_id"])?;
        let previous = &state["previous_version"];
        if !previous.is_null() && !is_version(previous.as_str()?) {
            return None;
        }
        iso_date(state["detected_at"].as_str()?)?;
    }
    let previous = result.get("previous")?;
    if !previous.is_null() && (!is_file_id(previous["file_id"].as_str()?) || !lower_hex(previous["sha256"].as_str()?, 64)) {
        return None;
    }
    Some(())
}

pub fn decode_index(data: &[u8], chat_id: &str, bot_id: &str) -> Result<Value> {
    serde_json::from_slice::<Value>(data)
        .ok()
        .filter(|result| valid_index(result, chat_id, bot_id).is_some())
        .ok_or_else(|| Error::telegram("Каталог повреждён или относится к другому боту/группе."))
}

fn private_group(chat: &Value) -> bool {
    matches!(chat["type"].as_str(), Some("group" | "supergroup")) && chat["username"].as_str().is_none_or(str::is_empty)
}

/// The pinned catalog: `file_id`, `sha256`, `message_id` and the decoded `index`; None when nothing is pinned.
pub fn current_catalog(client: &dyn Client, chat_id: &str) -> Result<Option<Value>> {
    let chat = client.call("getChat", json!({"chat_id": chat_id}))?;
    if !private_group(&chat) {
        return Err(Error::telegram("Для личных логов нужна закрытая группа без публичного @имени."));
    }
    let message = &chat["pinned_message"];
    if !message.is_object() {
        return Ok(None);
    }
    let name = message["document"]["file_name"].as_str().unwrap_or("");
    let good_name = name.strip_prefix("lc-index-").and_then(|rest| rest.strip_suffix(".json")).is_some_and(|id| lower_hex(id, 32));
    if message["from"]["id"] != client.bot_id() || !good_name {
        return Err(Error::telegram(
            "В группе закреплено другое сообщение. Выберите отдельную группу или восстановите последнее закрепление каталога LogCourier. Чужие закрепления не изменены.",
        ));
    }
    let file_id = message["document"]["file_id"].as_str().unwrap_or("");
    let data = client.download(file_id, INDEX_LIMIT)?;
    let digest = sha256_hex(&data);
    if message["caption"].as_str() != Some(&format!("{CATALOG_CAPTION}{digest}")) {
        return Err(Error::telegram("Контрольная сумма каталога не совпала."));
    }
    Ok(Some(json!({"file_id": file_id, "sha256": digest, "message_id": message["message_id"], "index": decode_index(&data, chat_id, client.bot_id())?})))
}

pub fn verify_connection(client: &dyn Client, chat_id: &str) -> Result<String> {
    let me = client.call("getMe", json!({}))?;
    if me["id"] != client.bot_id() {
        return Err(Error::telegram("ID бота не совпал с токеном."));
    }
    let chat = client.call("getChat", json!({"chat_id": chat_id}))?;
    if !private_group(&chat) {
        return Err(Error::telegram("Нужна закрытая группа Telegram."));
    }
    let bot: i64 = client.bot_id().parse().unwrap_or(0);
    let member = client.call("getChatMember", json!({"chat_id": chat_id, "user_id": bot}))?;
    if member["status"].as_str() != Some("administrator") || member["can_pin_messages"] != Value::Bool(true) {
        return Err(Error::telegram("Дайте боту право администратора «Закрепление сообщений»."));
    }
    let username = me["username"].as_str().map_or_else(|| client.bot_id().to_owned(), str::to_owned);
    let title = chat["title"].as_str().unwrap_or(chat_id);
    Ok(format!("@{username} → {title}"))
}

pub fn deliver(store: &Store, config: &Config, client: &dyn Client, cancelled: &Cancelled) -> Result<String> {
    if !config.consent {
        return Err(Error::value("Отправка текста логов не разрешена."));
    }
    if client.bot_id() != config.bot_id {
        return Err(Error::value("Токен относится к другому боту."));
    }
    let destination = config.destination();
    let key = format!("{PENDING_KEY}{destination}");
    checkpoint(cancelled)?;
    let head = current_catalog(client, &config.chat_id)?;
    if head.as_ref().is_some_and(|head| head["index"]["device_id"].as_str() != Some(&config.device_id)) {
        return Err(Error::telegram("Эта группа занята другим сборщиком. Используйте отдельную группу."));
    }
    let current = head.as_ref().and_then(|head| head["sha256"].as_str().map(str::to_owned));
    let mut pending = store.get(&key);
    if pending.as_ref().is_some_and(|pending| pending.get("sha256").is_none()) {
        // Left by a version that recognised the catalog by file_id; that identifier is already stale, so the interrupted
        // catalog is rebuilt from the queue instead.
        store.forget(&key)?;
        pending = None;
    }
    if let Some(pending) = pending {
        checkpoint(cancelled)?;
        finish_catalog(store, config, client, &pending, head.as_ref())?;
        return Ok("Каталог восстановлен; отправленные архивы повторно не загружались.".to_owned());
    }
    let mut known = store.get(&format!("{HEAD_DIGEST_KEY}{destination}")).and_then(|value| value.as_str().map(str::to_owned));
    if known.is_none() && store.get(&format!("{LEGACY_HEAD_KEY}{destination}")).is_some() {
        // Telegram hands out a new file_id for the same document after a while, so the identifier stored before 0.1.3
        // proves nothing beyond "a catalog was pinned": adopt whatever is pinned now, and keep refusing when nothing is.
        known = Some(current.clone().unwrap_or_default());
    }
    if known.is_some() && known != current {
        return Err(Error::telegram(LOST_PIN));
    }
    for row in store.queue(&destination)?.iter().take(DELIVERY_BATCH_LIMIT) {
        checkpoint(cancelled)?;
        if row.file_id.as_deref().is_some_and(|id| !id.is_empty()) {
            continue;
        }
        let message = client.send_document(&config.chat_id, &row.name, row.payload.as_deref().unwrap_or_default(), &caption(&row.metadata()))?;
        store.receipt(&row.id, message["document"]["file_id"].as_str().unwrap_or_default(), message["message_id"].as_i64().unwrap_or(0))?;
    }
    let mut entries = Vec::new();
    for row in store.queue(&destination)? {
        if row.file_id.as_deref().is_some_and(|id| !id.is_empty()) {
            let mut entry = row.metadata();
            entry["file_id"] = json!(row.file_id);
            entry["message_id"] = json!(row.message_id);
            entries.push(entry);
        }
        if entries.len() == CATALOG_MAX_ENTRIES {
            break;
        }
    }
    let versions = Value::Object(current_versions(store, config));
    let head_versions = head.as_ref().and_then(|head| head["index"].get("keyswitch_versions")).cloned().unwrap_or_else(|| json!({}));
    if entries.is_empty() && versions == head_versions {
        return Ok("Новых архивов для отправки нет.".to_owned());
    }
    checkpoint(cancelled)?;
    // Detect another writer before replacing the catalog pointer.
    let latest = current_catalog(client, &config.chat_id)?;
    if latest.as_ref().and_then(|latest| latest["sha256"].as_str().map(str::to_owned)) != current {
        return Err(Error::telegram("Каталог изменился во время отправки. Повторите позже."));
    }
    let previous = head.as_ref().map_or(Value::Null, |head| json!({"file_id": head["file_id"], "sha256": head["sha256"]}));
    let index = json!({
        "schema": 1, "kind": "logcourier.index", "device_id": config.device_id, "chat_id": config.chat_id, "bot_id": client.bot_id(),
        "created_at": iso_now(), "entries": entries, "keyswitch_versions": versions, "previous": previous,
    });
    let data = dumps(&index, false).into_bytes();
    if data.len() > INDEX_LIMIT {
        return Err(Error::telegram("Каталог слишком большой; отправка остановлена."));
    }
    let digest = sha256_hex(&data);
    let name = format!("lc-index-{}.json", new_id());
    checkpoint(cancelled)?;
    let message = client.send_document(&config.chat_id, &name, &data, &format!("{CATALOG_CAPTION}{digest}"))?;
    let ids: Vec<Value> = entries_ids(&index);
    let count = ids.len();
    let pending = json!({
        "sha256": digest, "message_id": message["message_id"], "ids": ids, "previous_sha256": current,
        "previous_message_id": head.as_ref().map_or(Value::Null, |head| head["message_id"].clone()),
    });
    store.set(&key, &pending)?;
    checkpoint(cancelled)?;
    finish_catalog(store, config, client, &pending, head.as_ref())?;
    Ok(format!("Отправлено и включено в каталог: {count} архивов."))
}

fn entries_ids(index: &Value) -> Vec<Value> {
    index["entries"].as_array().map_or_else(Vec::new, |entries| entries.iter().map(|entry| entry["bundle_id"].clone()).collect())
}

fn finish_catalog(store: &Store, config: &Config, client: &dyn Client, pending: &Value, head: Option<&Value>) -> Result<()> {
    let current = head.map_or(Value::Null, |head| head["sha256"].clone());
    if current != pending["previous_sha256"] && current != pending["sha256"] {
        return Err(Error::telegram("Закреплённый каталог изменён другим процессом. Восстановление остановлено."));
    }
    if current != pending["sha256"] {
        let result = client.call("pinChatMessage", json!({"chat_id": config.chat_id, "message_id": pending["message_id"], "disable_notification": true}))?;
        if result != Value::Bool(true) {
            return Err(Error::telegram("Telegram не подтвердил закрепление каталога."));
        }
        let fresh = current_catalog(client, &config.chat_id)?;
        if fresh.as_ref().is_none_or(|fresh| fresh["sha256"] != pending["sha256"]) {
            return Err(Error::telegram("Новый каталог не виден в закреплении. Проверьте права бота."));
        }
    }
    let ids: Vec<String> = pending["ids"].as_array().map_or_else(Vec::new, |ids| ids.iter().filter_map(|id| id.as_str().map(str::to_owned)).collect());
    // Do not unpin anything automatically: never disturb another participant's pins.
    store.acknowledge_index(&config.destination(), &ids, pending["sha256"].as_str().unwrap_or_default())
}

/// `keyswitch_version`: None for the whole history, `current` for each source's latest version, or a version.
pub fn list_entries(client: &dyn Client, chat_id: &str, limit: usize, pages: usize, keyswitch_version: Option<&str>) -> Result<Vec<Value>> {
    if keyswitch_version.is_some_and(|version| version != "current" && !is_version(version)) {
        return Err(Error::value("Укажите версию KeySwitch, например 0.16.2, или current."));
    }
    let Some(head) = current_catalog(client, chat_id)? else {
        return Ok(Vec::new());
    };
    let mut index = head["index"].clone();
    let versions = index.get("keyswitch_versions").cloned().unwrap_or_else(|| json!({}));
    if keyswitch_version == Some("current") && versions.as_object().is_none_or(serde_json::Map::is_empty) {
        return Err(Error::telegram(
            "В каталоге ещё не определена версия KeySwitch. Обновите и запустите сборщик; для старых и неверсионированных логов явно укажите --all-versions.",
        ));
    }
    let device_id = index["device_id"].clone();
    let mut result = Vec::new();
    let mut seen_files: HashSet<String> = HashSet::from([head["file_id"].as_str().unwrap_or_default().to_owned()]);
    let mut seen_bundles: HashSet<String> = HashSet::new();
    for _ in 0..pages {
        for entry in index["entries"].as_array().map_or(&[][..], Vec::as_slice).iter().rev() {
            if seen_bundles.insert(entry["bundle_id"].as_str().unwrap_or_default().to_owned()) {
                let version = entry["keyswitch_version"].as_str().filter(|text| !text.is_empty());
                let selected = match keyswitch_version {
                    None => true,
                    Some("current") => {
                        let source = entry["source_id"].as_str().unwrap_or_default();
                        version.is_some() && version == versions[source]["version"].as_str()
                    }
                    Some(wanted) => version == Some(wanted),
                };
                if entry["kind"].as_str() != Some(MARKER_KIND) && selected {
                    result.push(entry.clone());
                }
            }
            if result.len() >= limit {
                return Ok(result);
            }
        }
        let previous = index["previous"].clone();
        if !previous.is_object() {
            return Ok(result);
        }
        let file_id = previous["file_id"].as_str().unwrap_or_default();
        if !seen_files.insert(file_id.to_owned()) {
            return Err(Error::telegram("Циклическая ссылка в каталоге."));
        }
        let data = client.download(file_id, INDEX_LIMIT)?;
        if Some(sha256_hex(&data).as_str()) != previous["sha256"].as_str() {
            return Err(Error::telegram("Повреждена цепочка каталогов."));
        }
        index = decode_index(&data, chat_id, client.bot_id())?;
        if index["device_id"] != device_id {
            return Err(Error::telegram("Смешаны каталоги разных устройств."));
        }
    }
    Err(Error::telegram("Достигнут предел страниц каталога. Уменьшите запрашиваемое число файлов."))
}
