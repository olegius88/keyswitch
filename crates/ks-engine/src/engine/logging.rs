//! Everything the engine says goes to the log in one format, parsed by tools: `TECHNICAL` and one JSON object.

use super::{Alternatives, DetectionDecision, Value, EngineCore, FieldReader, logged, EngineResult, WordOptions, text_for_group, WaitingContextWord, ms, pystr, KeyEvent, is_navigation_key, Arc};
use ks_config::settings_diagnostics::settings_snapshot;
use ks_lang::WordScore;
use serde_json::{json, Map};

/// `_technical_event(event, **fields)`: the fields are built only when the log wants them.
macro_rules! technical_event {
    ($shared:expr, $event:expr $(, $key:literal : $value:expr)* $(,)?) => {{
        let shared: &$crate::engine::Shared = &$shared;
        if shared.technical_logging() {
            let mut payload = serde_json::Map::new();
            payload.insert("schema".to_owned(), serde_json::json!(1));
            payload.insert("event".to_owned(), serde_json::json!($event));
            $(payload.insert($key.to_owned(), serde_json::json!($value));)*
            shared.emit_technical(payload);
        }
    }};
}
pub(crate) use technical_event;

/// The keyword arguments of `log_word_evaluation`.
pub struct WordEvaluation<'a> {
    pub trigger: &'a str,
    pub original: &'a str,
    pub alternatives: &'a Alternatives,
    pub application: &'a str,
    pub enabled: bool,
    pub trigger_enabled: bool,
    pub manual_layout_protected: bool,
    pub application_excluded: bool,
    pub decision: Option<&'a DetectionDecision>,
    pub caret_unknown: bool,
    pub protection: Option<Value>,
    pub source_group: Option<i64>,
    pub early_switch_origin: Option<i64>,
    pub idle_ms: Option<i64>,
    pub context: Option<(Alternatives, Option<i64>)>,
    pub literal_head: &'a str,
}

fn group_map(texts: &[(i64, String)]) -> Value {
    Value::Object(texts.iter().map(|(group, text)| (group.to_string(), json!(text))).collect())
}

/// `sysconfig.get_platform()`: one version ships a build per architecture.
fn build_platform() -> String {
    build_platform_of(std::env::consts::OS, std::env::consts::ARCH)
}

fn build_platform_of(os: &str, arch: &str) -> String {
    match (os, arch) {
        ("windows", "x86_64") => "win-amd64".to_owned(),
        ("windows", "aarch64") => "win-arm64".to_owned(),
        ("windows", "x86") => "win32".to_owned(),
        (os, arch) => format!("{os}-{arch}"),
    }
}

impl EngineCore {
    pub fn technical_session_event(&self, event: &str) {
        technical_event!(
            self.shared, event,
            "keyswitch_version": ks_config::logsetup::VERSION,
            // The platform the build's Python was made for (win-amd64, win-arm64, win32, ...): one
            // version ships a build per architecture, and a log has to say which one it came from.
            "build_platform": build_platform(),
            "backend": self.backend_label,
            "intent_model": self.intent_model_status.as_dict(),
            "context_model_status": self.context_policy.status,
            "field_reader_status": self.field_reader_status(),
            "field_reader_details": self.field_reader_details(),
            "language_models": Value::Object(self.models.iter().enumerate().map(|(group, model)| (group.to_string(), json!({
                "locale": model.locale,
                "words": model.frequencies.len(),
                "source": model.source,
            }))).collect()),
            "settings": Value::Object(settings_snapshot(&self.settings)),
        );
    }

    fn _score_diagnostics(score: &WordScore) -> Value {
        json!({
            "value": logged(score.value),
            "known": score.known,
            "frequency": score.frequency,
            "exact": score.exact,
            "spell_known": score.spell_known,
            "ngram_score": logged(score.ngram_score),
            "invalid_ratio": logged(score.invalid_ratio),
        })
    }

    #[must_use]
    pub fn _decision_diagnostics(decision: &DetectionDecision) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("should_convert".into(), json!(decision.should_convert));
        map.insert("replacement".into(), json!(decision.replacement));
        map.insert("source_group".into(), json!(decision.source_group));
        map.insert("target_group".into(), json!(decision.target_group));
        map.insert("confidence".into(), json!(logged(decision.confidence)));
        map.insert("reason".into(), json!(decision.reason));
        map.insert("source_score".into(), Self::_score_diagnostics(&decision.source_score));
        map.insert("target_score".into(), Self::_score_diagnostics(&decision.target_score));
        map.insert("model_probability".into(), json!(decision.model_probability));
        map.insert("model_threshold".into(), json!(decision.model_threshold));
        map.insert("model_version".into(), json!(decision.model_version));
        map
    }

    pub fn log_word_evaluation(&mut self, evaluation: WordEvaluation<'_>) -> EngineResult<()> {
        if !self.shared.technical_logging() {
            return Ok(());
        }
        let WordEvaluation {
            trigger, original, alternatives, application, enabled, trigger_enabled, manual_layout_protected, application_excluded,
            decision, caret_unknown, protection, source_group, early_switch_origin, idle_ms, context, literal_head,
        } = evaluation;
        let mut decision_payload = decision.map(Self::_decision_diagnostics);
        // Never put text typed inside an excluded application into the log,
        // even when detailed diagnostics are explicitly enabled.
        let logged_original = if application_excluded { "<redacted>" } else { original };
        let logged_alternatives = if application_excluded { json!({}) } else { group_map(alternatives) };
        if application_excluded && let Some(payload) = decision_payload.as_mut() {
            payload.insert("replacement".into(), json!("<redacted>"));
        }
        let skipped_reason = if application_excluded {
            Some("application_excluded")
        } else if !enabled {
            Some("disabled")
        } else if !trigger_enabled {
            Some("trigger_disabled")
        } else if manual_layout_protected {
            Some("manual_layout_protected")
        } else if caret_unknown {
            Some("caret_moved")
        } else {
            None
        };
        // When the detector was not consulted, still record what it would have
        // said so that a missed correction can be told from a wrong verdict.
        let mut shadow_payload = None;
        if decision.is_none()
            && skipped_reason.is_some()
            && skipped_reason != Some("application_excluded")
            && let Some(group) = source_group
            && self.in_models(group)
        {
            let trigger_kind = ks_lang::intent_model::CorrectionTrigger::parse(trigger).unwrap_or(ks_lang::intent_model::CorrectionTrigger::Space);
            let shadow = self.decide_word(original, alternatives, group, application, trigger_kind, WordOptions::default())?;
            shadow_payload = Some(Self::_decision_diagnostics(&shadow));
        }
        let (context_words, context_group) = match context {
            Some(context) => context,
            None => self.context_for(application),
        };
        technical_event!(
            self.shared, "word_evaluation",
            "trigger": trigger,
            "original": logged_original,
            "literal_head": if application_excluded && !literal_head.is_empty() { "<redacted>" } else { literal_head },
            "alternatives": logged_alternatives,
            "source_group": source_group,
            "application": application,
            "enabled": enabled,
            "trigger_enabled": trigger_enabled,
            "manual_layout_protected": manual_layout_protected,
            "caret_unknown": caret_unknown,
            "protection": protection,
            "application_excluded": application_excluded,
            "skipped_reason": skipped_reason,
            "minimum_length": self.minimum_word_length(),
            "confidence_threshold": self.confidence_threshold(),
            "context": json!({
                "group": context_group,
                "words": if application_excluded { json!({}) } else { group_map(&context_words) },
            }),
            "early_switch_origin": early_switch_origin,
            "idle_ms": idle_ms,
            "learning": self.learning_diagnostics(source_group, original),
            "decision": decision_payload,
            "shadow_decision": shadow_payload,
        );
        Ok(())
    }

    /// Record a scheduled correction that is thrown away before it runs.
    ///
    /// A correction waits for the release of the key that triggered it, so a
    /// shortcut or a focus change in between silently cancels it: from the
    /// outside the hotkey simply did nothing.
    pub fn log_pending_dropped(&self, reason: &str) {
        let Some(plan) = &self.pending else {
            return;
        };
        let excluded = self.application_excluded(&plan.application);
        technical_event!(
            self.shared, "pending_correction_dropped",
            "reason": if reason.is_empty() { "unspecified" } else { reason },
            "mode": plan.mode,
            "original": if excluded { "<redacted>" } else { plan.original.as_str() },
            "replacement": if excluded { "<redacted>" } else { plan.replacement.as_str() },
            "source_group": plan.source_group,
            "target_group": plan.target_group,
            "trigger_keycode": self.pending_trigger_keycode,
            "application": plan.application,
            "application_excluded": excluded,
        );
    }

    /// Which of the user's rules decides about this word, if any.
    ///
    /// Without it a log line cannot tell a correction the user's rule forced, or
    /// a word their rule kept, from one the model decided on its own.
    pub fn learning_diagnostics(&self, source_group: Option<i64>, word: &str) -> Value {
        let enabled = self.flag("detection.learning", true);
        let Some(group) = source_group else {
            return json!({"enabled": enabled});
        };
        let rule = if enabled { self.learning.r#match(group, word) } else { None };
        let mut rejected = self.rejected_targets(group, word);
        rejected.sort_unstable();
        json!({
            "enabled": enabled,
            "rule": rule.map(|rule| json!({
                "pattern": rule.pattern,
                "match": rule.r#match.as_str(),
                "case_sensitive": rule.case_sensitive,
                "action": rule.action.as_str(),
                "target_group": rule.target_group,
            })),
            "forced_target": self.forced_target_group(group, word),
            "rejected_targets": rejected,
        })
    }

    /// Record a word that was thrown away before it could be corrected.
    pub fn log_word_discarded(&self, reason: &str) {
        if self.strokes.is_empty() && self.symbol_strokes.is_empty() {
            return;
        }
        // Read the setting first: the application probe is a system call.
        if !self.shared.technical_logging() {
            return;
        }
        let application = self.backend.active_application();
        let excluded = self.application_excluded(&application);
        let original = format!(
            "{}{}",
            text_for_group(&self.symbol_strokes, self.source_group),
            text_for_group(&self.strokes, self.source_group)
        );
        technical_event!(
            self.shared, "word_discarded",
            "reason": if reason.is_empty() { "unspecified" } else { reason },
            "original": if excluded { "<redacted>" } else { original.as_str() },
            "length": self.strokes.len(),
            "symbol_count": self.symbol_strokes.len(),
            "source_group": self.source_group,
            "application": application,
            "application_excluded": excluded,
        );
    }

    pub fn log_context_wait(&self, event: &str, waiting: Option<&WaitingContextWord>, reason: &str, fields: &[(&str, Value)]) {
        let Some(waiting) = waiting else {
            return;
        };
        if !self.shared.technical_logging() {
            return;
        }
        let remaining = ms(waiting.deadline - self.monotonic()).max(0);
        let mut payload = Map::new();
        payload.insert("schema".into(), json!(1));
        payload.insert("event".into(), json!(event));
        payload.insert("wait_id".into(), json!(waiting.diagnostic_id));
        payload.insert("reason".into(), json!(reason));
        payload.insert("word_characters".into(), json!(pystr::len(&waiting.plan.original)));
        payload.insert("source_group".into(), json!(waiting.plan.source_group));
        payload.insert("target_group".into(), json!(waiting.plan.target_group));
        payload.insert("remaining_ms".into(), json!(remaining));
        for (name, value) in fields {
            payload.insert((*name).to_owned(), value.clone());
        }
        self.shared.emit_technical(payload);
    }

    pub fn log_input_edit(&self, event: &KeyEvent, application: &str) {
        // Observe editing controls, not printable keystrokes or field contents.
        if !self.shared.technical_logging() || !self.flag("enabled", true) || self.application_excluded(application) {
            return;
        }
        let edit = if event.control() || event.alt() || event.super_key() {
            "shortcut"
        } else if event.key_name == "BackSpace" {
            "backspace"
        } else if event.key_name == "Delete" {
            "delete"
        } else if event.key_name == "Pointer" {
            "pointer"
        } else if is_navigation_key(&event.key_name) {
            "navigation"
        } else {
            return;
        };
        technical_event!(
            self.shared, "input_edit_observed", "edit": edit, "application": application,
            "group": event.group, "tracked_word_characters": self.strokes.len(),
            "context_characters_before": pystr::len(&self.context_policy.stream.text),
            "last_committed_available": self.last_committed.is_some() && !self.last_committed_stale,
            "wait_id": self.context_waiting.as_ref().map(|waiting| waiting.diagnostic_id),
            "text_verified": false,
        );
    }

    pub fn field_reader_status(&self) -> String {
        let Some(reader) = &self.context_policy.reader else {
            return "not_configured".to_owned();
        };
        match &self.platform_reader {
            Some(platform) if std::ptr::addr_eq(Arc::as_ptr(reader), Arc::as_ptr(platform)) => platform.status(),
            _ => "custom_reader".to_owned(),
        }
    }

    pub fn field_reader_details(&self) -> Value {
        if let (Some(reader), Some(platform)) = (&self.context_policy.reader, &self.platform_reader)
            && std::ptr::addr_eq(Arc::as_ptr(reader), Arc::as_ptr(platform))
        {
            let mut details = platform.diagnostics();
            details.insert("retry".into(), Value::Object(platform.retry_diagnostics()));
            return Value::Object(details);
        }
        json!({"status": self.field_reader_status()})
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    #[test]
    fn a_build_names_its_platform_as_python_does() {
        assert_eq!(super::build_platform_of("windows", "x86_64"), "win-amd64");
        assert_eq!(super::build_platform_of("windows", "aarch64"), "win-arm64");
        assert_eq!(super::build_platform_of("windows", "x86"), "win32");
        assert_eq!(super::build_platform_of("linux", "x86_64"), "linux-x86_64");
    }
}
