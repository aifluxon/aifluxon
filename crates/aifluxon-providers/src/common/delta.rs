#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeltaMode {
    Incremental,
    CumulativeCompatible,
}

#[derive(Default)]
pub struct TextDeltaReconciler {
    emitted: String,
    cumulative_detected: bool,
}

impl TextDeltaReconciler {
    pub fn push(&mut self, value: &str, mode: DeltaMode) -> Option<String> {
        match mode {
            DeltaMode::Incremental => self.push_compatible(value, false, false),
            DeltaMode::CumulativeCompatible => self.push_compatible(value, false, true),
        }
    }

    pub fn push_compatible(
        &mut self,
        incoming: &str,
        explicit_snapshot: bool,
        allow_cumulative_detection: bool,
    ) -> Option<String> {
        if incoming.is_empty() {
            return None;
        }

        let looks_cumulative = allow_cumulative_detection
            && !self.emitted.is_empty()
            && incoming.starts_with(&self.emitted);
        if explicit_snapshot || self.cumulative_detected || looks_cumulative {
            self.cumulative_detected = true;
            let suffix = missing_complete_text_suffix(&self.emitted, incoming)?;
            self.emitted.push_str(&suffix);
            return Some(suffix);
        }

        self.emitted.push_str(incoming);
        Some(incoming.to_string())
    }

    pub fn complete(&mut self, complete: &str) -> Result<Option<String>, String> {
        let suffix = strict_complete_text_suffix(&self.emitted, complete)?;
        if let Some(suffix) = &suffix {
            self.emitted.push_str(suffix);
            self.cumulative_detected = true;
        }
        Ok(suffix)
    }

    pub fn emitted(&self) -> &str {
        &self.emitted
    }

    pub fn cumulative_detected(&self) -> bool {
        self.cumulative_detected
    }
}

pub fn missing_complete_text_suffix(current: &str, complete: &str) -> Option<String> {
    if complete.is_empty() || current == complete || current.ends_with(complete) {
        return None;
    }
    if let Some(suffix) = complete.strip_prefix(current) {
        return (!suffix.is_empty()).then(|| suffix.to_string());
    }

    let overlap = (1..=current.len().min(complete.len()))
        .rev()
        .find(|overlap| {
            current.is_char_boundary(current.len() - overlap)
                && complete.is_char_boundary(*overlap)
                && current[current.len() - overlap..] == complete[..*overlap]
        })
        .unwrap_or(0);
    let suffix = &complete[overlap..];
    (!suffix.is_empty()).then(|| suffix.to_string())
}

pub fn strict_complete_text_suffix(
    current: &str,
    complete: &str,
) -> Result<Option<String>, String> {
    if complete.is_empty() || current == complete {
        return Ok(None);
    }
    if let Some(suffix) = complete.strip_prefix(current) {
        return Ok((!suffix.is_empty()).then(|| suffix.to_string()));
    }
    Err("Responses stream text diverged from its completed snapshot.".to_string())
}

pub fn reconcile_terminal_text(
    current: &str,
    complete: &str,
    saw_output_done: bool,
) -> Result<Option<String>, String> {
    if saw_output_done && current != complete {
        return Err(
            "Responses done output diverged from its completed terminal snapshot.".to_string(),
        );
    }
    strict_complete_text_suffix(current, complete)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cumulative_snapshots_emit_only_missing_suffix_and_duplicates_emit_nothing() {
        let mut state = TextDeltaReconciler::default();
        assert_eq!(
            state.push("hello", DeltaMode::CumulativeCompatible),
            Some("hello".to_string())
        );
        assert_eq!(state.push("hello", DeltaMode::CumulativeCompatible), None);
        assert_eq!(
            state.push("hello world", DeltaMode::CumulativeCompatible),
            Some(" world".to_string())
        );
        assert_eq!(state.emitted(), "hello world");
    }

    #[test]
    fn incremental_repeated_text_remains_incremental() {
        let mut state = TextDeltaReconciler::default();
        assert_eq!(
            state.push("ha", DeltaMode::Incremental),
            Some("ha".to_string())
        );
        assert_eq!(
            state.push("ha", DeltaMode::Incremental),
            Some("ha".to_string())
        );
        assert_eq!(state.emitted(), "haha");
        assert!(!state.cumulative_detected());
    }

    #[test]
    fn custom_provider_cumulative_deltas_only_append_the_missing_suffix() {
        let mut state = TextDeltaReconciler::default();
        for incoming in ["模型", "模型正在", "模型正在输出"] {
            state.push(incoming, DeltaMode::CumulativeCompatible);
        }
        assert_eq!(state.emitted(), "模型正在输出");
        assert!(state.cumulative_detected());
    }

    #[test]
    fn overlapping_complete_suffix_is_appended_once() {
        assert_eq!(
            missing_complete_text_suffix("你好，", "你好，世界"),
            Some("世界".to_string())
        );
        assert_eq!(
            missing_complete_text_suffix("partial text", "text and more"),
            Some(" and more".to_string())
        );
        assert_eq!(missing_complete_text_suffix("完整回答", "完整回答"), None);
    }

    #[test]
    fn strict_done_snapshots_reject_divergence() {
        let mut part = TextDeltaReconciler::default();
        assert_eq!(
            part.push_compatible("先分析", false, false),
            Some("先分析".to_string())
        );
        assert_eq!(
            part.complete("先分析，再验证").unwrap(),
            Some("，再验证".to_string())
        );
        assert_eq!(part.emitted(), "先分析，再验证");
        assert_eq!(part.complete("先分析，再验证").unwrap(), None);
        assert!(part.complete("另一份思考").is_err());
        assert!(reconcile_terminal_text("F", "Fextra", true).is_err());
        assert_eq!(
            reconcile_terminal_text("F", "Fextra", false).unwrap(),
            Some("extra".to_string())
        );
    }
}
