//! Clearing the group and starting a new chain of catalogs.
//!
//! The Bot API gives a bot no way to read a group's history, but in a supergroup message identifiers run upward from 1.
//! The collector posts one short notice to learn the newest identifier, takes every pin down and forgets the chain of
//! catalogs it was continuing, then asks Telegram to delete every message up to the notice, a batch at a time from the
//! newest. The progress lives in the store: an interrupted clearing goes on from the batch it stopped at, and since the
//! pins are taken down only once, a chain started in the meantime keeps its pin.
//!
//! A bot may delete only messages younger than two days (Bot API, deleteMessage), and Telegram refuses a whole batch when
//! it keeps even one of its messages, deleting none of them. Messages are numbered in the order they were sent, so the
//! kept ones are all older than the deleted ones: batches go from the newest, the first refused batch is searched by
//! halving for where the deletable part starts, and nothing older is asked for.
use ks_cast::To as _;
use serde_json::{Value, json};

use crate::catalog::{checkpoint, current_catalog};
use crate::config::Config;
use crate::constants::*;
use crate::error::{Error, Result};
use crate::rate_limit::Cancelled;
use crate::store::{RESET_KEY, Store};
use crate::telegram::Client;

const NOTICE: &str = "LogCourier очищает группу и начинает каталог заново.";
const UNCHANGED: &str = " Группа не изменена.";

/// Start a new chain of catalogs; with `delete_messages`, delete the group's messages first.
/// A clearing left unfinished is continued, whatever `delete_messages` says now.
pub fn reset_group(store: &Store, config: &Config, client: &dyn Client, delete_messages: bool, cancelled: &Cancelled, progress: &dyn Fn(&str)) -> Result<String> {
    if client.bot_id() != config.bot_id {
        return Err(Error::value("Токен относится к другому боту."));
    }
    if store.get(&format!("{RESET_KEY}{}", config.destination())).is_none() {
        restart(store, config, client, delete_messages, cancelled)?;
        if !delete_messages {
            return Ok("Каталог начнётся заново со следующей отправки. Закрепления в группе сняты, сообщения не удалялись.".to_owned());
        }
    }
    clear(store, config, client, cancelled, progress)
}

fn restart(store: &Store, config: &Config, client: &dyn Client, delete_messages: bool, cancelled: &Cancelled) -> Result<()> {
    checkpoint(cancelled)?;
    let chat = client.call("getChat", json!({"chat_id": config.chat_id}))?;
    if delete_messages && chat["type"].as_str() != Some("supergroup") {
        return Err(Error::telegram(format!(
            "Удалить все сообщения бот может только в супергруппе. Очистите группу вручную и начните каталог заново без удаления сообщений.{UNCHANGED}"
        )));
    }
    let bot: i64 = client.bot_id().parse().unwrap_or(0);
    let member = client.call("getChatMember", json!({"chat_id": config.chat_id, "user_id": bot}))?;
    if member["status"].as_str() != Some("administrator") || member["can_pin_messages"] != Value::Bool(true) {
        return Err(Error::telegram(format!("Дайте боту право администратора «Закрепление сообщений».{UNCHANGED}")));
    }
    if delete_messages && member["can_delete_messages"] != Value::Bool(true) {
        return Err(Error::telegram(format!("Чтобы очистить группу, дайте боту право администратора «Удаление сообщений».{UNCHANGED}")));
    }
    let head = current_catalog(client, &config.chat_id)?;
    if head.as_ref().is_some_and(|head| head["index"]["device_id"].as_str() != Some(&config.device_id)) {
        return Err(Error::telegram(format!("Каталог в этой группе ведёт другой сборщик.{UNCHANGED}")));
    }
    if head.is_none() && !store.delivered(&config.destination())? {
        // Nothing ties this group to the collector: it may be the wrong one, so leave it alone.
        return Err(Error::value(format!("Этот сборщик ещё ничего не отправлял в эту группу.{UNCHANGED}")));
    }
    let mut clearing = None;
    if delete_messages {
        checkpoint(cancelled)?;
        let notice = client.call("sendMessage", json!({"chat_id": config.chat_id, "text": NOTICE, "disable_notification": true}))?;
        let confirmed = notice["chat"]["id"].as_i64().is_some() && notice["chat"]["id"].as_i64() == config.chat_id.parse::<i64>().ok();
        let top = notice["message_id"].as_i64().filter(|_| confirmed).ok_or_else(|| Error::telegram("Telegram не подтвердил сообщение в выбранной группе."))?;
        clearing = Some(json!({"top": top, "next": top, "kept": null}));
    }
    checkpoint(cancelled)?;
    if client.call("unpinAllChatMessages", json!({"chat_id": config.chat_id}))? != Value::Bool(true) {
        return Err(Error::telegram("Telegram не подтвердил снятие закреплений. Каталог не изменён."));
    }
    store.restart_chain(&config.destination(), clearing.as_ref())?;
    Ok(())
}

fn clear(store: &Store, config: &Config, client: &dyn Client, cancelled: &Cancelled, progress: &dyn Fn(&str)) -> Result<String> {
    let key = format!("{RESET_KEY}{}", config.destination());
    let mut state = store.get(&key).unwrap_or_else(|| json!({"top": 0, "next": 0, "kept": null}));
    let top = state["top"].as_i64().unwrap_or(0);
    let mut steps = || -> Result<()> {
        while state["next"].as_i64().unwrap_or(0) >= FIRST_REGULAR_MESSAGE_ID {
            checkpoint(cancelled)?;
            let last = state["next"].as_i64().unwrap_or(0);
            let first = FIRST_REGULAR_MESSAGE_ID.max(last - DELETE_BATCH_MESSAGES + 1);
            let start = deleted_from(client, &config.chat_id, first, last, cancelled)?;
            if start > first {
                // Every message before a kept one is older still, so Telegram keeps it as well.
                state["next"] = json!(FIRST_REGULAR_MESSAGE_ID - 1);
                state["kept"] = json!(start - 1);
            } else {
                state["next"] = json!(first - 1);
            }
            store.set(&key, &state)?;
            progress(&format!("Очистка группы: обработаны сообщения с №{start} по №{top}."));
        }
        Ok(())
    };
    match steps() {
        Err(Error::Cancelled(_)) => return Err(Error::Cancelled("Очистка группы прервана; она продолжится с того же места.".to_owned())),
        Err(error) => return Err(error),
        Ok(()) => {}
    }
    // A clearing begun by 0.1.4 dropped its uncatalogued archives up front and counted them.
    let dropped = (store.finish_clearing(&config.destination(), state["kept"].as_i64())?).to::<i64>() + state["dropped"].as_i64().unwrap_or(0);
    Ok(summary(&state, dropped))
}

/// Delete messages first..=last; returns where the part Telegram deleted starts.
pub fn deleted_from(client: &dyn Client, chat_id: &str, first: i64, last: i64, cancelled: &Cancelled) -> Result<i64> {
    if delete(client, chat_id, first, last)? {
        return Ok(first);
    }
    // The batch as a whole was refused. Deleting from `start` to `last` succeeds exactly when `start` is past the kept
    // messages, so the smallest such `start` is found by halving.
    let (mut low, mut high) = (first + 1, last + 1);
    while low < high {
        checkpoint(cancelled)?;
        let middle = (low + high) / 2;
        if delete(client, chat_id, middle, last)? {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    Ok(low)
}

fn delete(client: &dyn Client, chat_id: &str, first: i64, last: i64) -> Result<bool> {
    let ids: Vec<i64> = (first..=last).collect();
    match client.call("deleteMessages", json!({"chat_id": chat_id, "message_ids": ids})) {
        Err(Error::Telegram { status_code: TELEGRAM_BAD_REQUEST_STATUS, .. }) => Ok(false),
        Err(error) => Err(error),
        Ok(Value::Bool(true)) => Ok(true),
        Ok(_) => Err(Error::telegram("Telegram не подтвердил удаление сообщений.")),
    }
}

fn plural(count: i64, forms: [&str; 3]) -> String {
    let (last, last_two) = (count.abs() % 10, count.abs() % 100);
    let form = if last == 1 && last_two != 11 {
        forms[0]
    } else if (2..=4).contains(&last) && !(12..=14).contains(&last_two) {
        forms[1]
    } else {
        forms[2]
    };
    format!("{count} {form}")
}

fn summary(state: &Value, dropped: i64) -> String {
    let top = state["top"].as_i64().unwrap_or(0);
    let mut text = match state["kept"].as_i64() {
        Some(kept) => format!(
            "Группа очищена частично: удалены сообщения с №{} по №{top}. Более ранние, до №{kept}, Telegram удалить не дал: бот может удалять только сообщения, отправленные за последние {}. Их можно удалить вручную.",
            kept + 1,
            plural(BOT_DELETE_WINDOW_HOURS, ["час", "часа", "часов"]),
        ),
        None => format!("Группа очищена: удалены все сообщения до №{top}, кроме служебного о создании группы."),
    };
    if dropped != 0 {
        text.push_str(&format!(" Вместе с ними удалены архивы, ещё не внесённые в каталог: {}.", plural(dropped, ["архив", "архива", "архивов"])));
    }
    text + " Новый каталог начнётся со следующей отправки."
}
