//! A word the engine holds for later (waiting for its neighbour, kept, converted provisionally, converted at a pause)
//! grows old in every way while the person types on: each way must lead to a word left alone, never to a stop.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::Arc;

use ks_backend::KeyEvent;
use ks_backend::constants::SHIFT_MASK;
use ks_engine::engine::PlanRef;
use ks_replay::harness::{Bundle, KEYS, PAUSE_KEYCODE, Session};

const STARTS: &[&str] = &["", "начало ", "start ", "Привет, как дела? ", "git ", "x = 2 "];
const FIRST: &[&str] = &[
    "tot", "ещ", "ещу", "lf", "yt", "yf", "ytn", "gj", "dct", "rfr", "z", "я", "d", "в", "r", "к", "f", "а", "b", "и", "c", "с", "x", "ч", "qr", "йк", "pr", "зк", "rp", "кз", "the", "of", "ghbdtn", "привет", "hello", "руддщ",
    "v67", "м67", "2ю", "webjs", "htmx", "git", "npm", "тзь", "ntcn", "тест", "кто-то", "rnj-nj", "mongo-cthdbcf", "ghbdtn-vbh", "DKRCV", "ЗОР",
];
const NEXT: &[&str] = &["d", "в", "z", "я", "ghbdtn", "привет", "the", "hello", "руддщ", "кто-то", "v67", "2ю", "ytn", "tot", "ещ", "lj,hsq", "a.b", "ф.и", "'", "-", ",", "б", "ю", ""];
const BOUNDARIES: &[&str] = &[" ", ",", ".", "enter", "tab", "deferred enter", "deferred tab", "pause", "idle"];

const AGES: usize = 9;

fn changed(plan: &PlanRef, age: usize) -> PlanRef {
    let mut plan = (**plan).clone();
    match age {
        2 => "another program".clone_into(&mut plan.application),
        3 => plan.boundary = None,
        4 => std::mem::swap(&mut plan.source_group, &mut plan.target_group),
        5 => plan.strokes.clear(),
        _ => {}
    }
    Arc::new(plan)
}

/// One way for what is held to grow old: 0 none, 1 another window, 2 and up what it was made of.
fn age(session: &mut Session, age: usize) {
    let core = &mut session.core;
    if let Some(waiting) = &mut core.context_waiting {
        match age {
            0 => {}
            1 => waiting.window = -1,
            6 => waiting.deadline = 0.0,
            7 => waiting.baseline = None,
            8 => waiting.settles_on_pause = !waiting.settles_on_pause,
            _ => waiting.plan = changed(&waiting.plan, age),
        }
    }
    for word in [&mut core.provisional, &mut core.paused_conversion].into_iter().flatten() {
        match age {
            0 | 7 | 8 => {}
            1 => word.window = -1,
            6 => word.deadline = 0.0,
            _ => word.plan = changed(&word.plan, age),
        }
    }
    core.kept = core
        .kept
        .iter()
        .map(|word| {
            let mut word = (**word).clone();
            match age {
                0 => {}
                1 => word.window = -1,
                6 => word.deadline = 0.0,
                7 => word.baseline = None,
                8 => word.field = None,
                _ => word.plan = changed(&word.plan, age),
            }
            Arc::new(word)
        })
        .collect();
    if age == 8 {
        core.kept = core
            .kept
            .iter()
            .map(|word| {
                let mut word = (**word).clone();
                word.segmentation_open = !word.segmentation_open;
                Arc::new(word)
            })
            .collect();
    }
}

fn type_text(session: &mut Session, text: &str) {
    for character in text.chars() {
        let Some((key, group)) = KEYS.iter().find_map(|key| Some((key, key.characters.iter().position(|typed| *typed == character)?))) else {
            continue;
        };
        if usize::try_from(session.backend.group()).ok() != Some(group) {
            session.select_layout(i32::try_from(group).expect("two layouts"));
        }
        session.tap(KeyEvent {
            pressed: true,
            keycode: key.keycode,
            key_name: character.to_string(),
            character: character.to_string(),
            characters: key.strings(),
            group: session.backend.group(),
            state: if key.shift { SHIFT_MASK } else { 0 },
            ..KeyEvent::default()
        });
    }
}

fn end_word(session: &mut Session, boundary: &str) {
    let group = session.backend.group();
    let named = |name: &str, deferred: bool| KeyEvent { pressed: true, key_name: name.to_owned(), characters: vec![String::new(), String::new()], deferred, group, ..KeyEvent::default() };
    match boundary {
        "enter" => session.tap(named("Return", false)),
        "tab" => session.tap(named("Tab", false)),
        "deferred enter" => session.handle(named("Return", true)),
        "deferred tab" => session.handle(named("Tab", true)),
        "pause" => session.tap(KeyEvent { keycode: PAUSE_KEYCODE, ..named("Pause", false) }),
        "idle" => session.idle(1.0),
        other => type_text(session, other),
    }
}

fn holds(session: &Session) -> bool {
    let core = &session.core;
    core.context_waiting.is_some() || core.provisional.is_some() || core.paused_conversion.is_some() || !core.kept.is_empty()
}

#[test]
fn what_is_held_grows_old_in_every_way() {
    let mut bundle = Bundle::load(None);
    let mut held = Vec::new();
    for start in STARTS {
        for first in FIRST {
            for closing in [" ", "idle"] {
                let mut session = bundle.session("firefox", 0, start.chars().collect());
                session.recovers = true;
                type_text(&mut session, first);
                end_word(&mut session, closing);
                if holds(&session) {
                    held.push((*start, *first, closing));
                }
                bundle.give_back(session);
            }
        }
    }
    assert!(!held.is_empty());
    let mut tried = 0;
    for (start, first, closing) in held.iter().step_by(held.len().div_ceil(40)) {
        for way in 0..AGES {
            for next in NEXT {
                for boundary in BOUNDARIES {
                    let mut session = bundle.session("firefox", 0, start.chars().collect());
                    session.recovers = true;
                    type_text(&mut session, first);
                    end_word(&mut session, closing);
                    age(&mut session, way);
                    type_text(&mut session, next);
                    end_word(&mut session, boundary);
                    tried += 1;
                    assert!(session.backend.caret() <= session.backend.text().len());
                    bundle.give_back(session);
                }
            }
        }
    }
    println!("{} held, {tried} tried", held.len());
    bundle.clean();
}
