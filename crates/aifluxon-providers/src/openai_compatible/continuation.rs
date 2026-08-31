use super::ApiFamily;
use crate::codex;
use aifluxon_core::{ContinuationReason, ModelTurn, ProviderTerminal};

pub fn apply_turn_continuation(family: ApiFamily, tools_enabled: bool, turn: &mut ModelTurn) {
    if !turn.tool_calls.is_empty() || turn.terminal == ProviderTerminal::ToolCalls {
        return;
    }
    if turn.terminal.continuation_reason().is_some() {
        return;
    }
    if family == ApiFamily::Codex && codex::should_continue_end_turn(&turn.opaque) {
        turn.terminal = ProviderTerminal::Continue(ContinuationReason::ProviderRequested);
        return;
    }
    if tools_enabled && promised_tool_work_without_call(&turn.text) {
        turn.terminal = ProviderTerminal::Continue(ContinuationReason::Incomplete);
    }
}

pub fn promised_tool_work_without_call(content: &str) -> bool {
    let text = strip_leading_acknowledgement(content.trim());
    if text.is_empty() {
        return false;
    }

    let lower = text.to_ascii_lowercase();
    let english_intent_markers = ["let me ", "i'll ", "i will ", "i'm going to "];
    for intent_marker in english_intent_markers {
        for (intent_index, _) in lower.match_indices(intent_marker) {
            if !intent_is_in_opening_clause(&lower, intent_index) {
                continue;
            }
            let clause = prospective_tool_clause(&lower[intent_index..]);
            if english_clause_promises_tool_work(clause) {
                return true;
            }
        }
    }

    let intent_markers = ["我先", "我会先", "我将先", "接下来我"];
    let tool_action_markers = [
        "查找",
        "搜索",
        "读取",
        "查看",
        "检查",
        "定位",
        "运行",
        "验证",
        "了解",
        "执行命令",
        "用 shell",
        "使用 shell",
        "用 rg",
        "使用 rg",
        "get-content",
        "select-string",
    ];

    for intent_marker in intent_markers {
        for (intent_index, _) in text.match_indices(intent_marker) {
            if !intent_is_in_opening_clause(text, intent_index) {
                continue;
            }
            let clause = prospective_tool_clause(&text[intent_index..]);
            if chinese_clause_promises_tool_work(clause, &tool_action_markers) {
                return true;
            }
        }
    }

    let bare_preamble = [
        "先看一下",
        "先查一下",
        "先检查",
        "先定位",
        "先读取",
        "先搜索",
        "先运行",
        "先验证",
        "先执行命令",
        "先用 shell",
        "先使用 shell",
        "先用 rg",
        "先使用 rg",
        "看一下",
        "查一下",
        "检查一下",
        "定位一下",
    ]
    .iter()
    .any(|marker| text.starts_with(marker));
    bare_preamble
        && chinese_clause_promises_tool_work(prospective_tool_clause(text), &tool_action_markers)
}

fn intent_is_in_opening_clause(text: &str, intent_index: usize) -> bool {
    !text[..intent_index].chars().any(is_tool_clause_boundary)
}

fn prospective_tool_clause(text: &str) -> &str {
    let end = text
        .char_indices()
        .find_map(|(index, character)| {
            (index > 0 && is_tool_clause_boundary(character)).then_some(index)
        })
        .unwrap_or(text.len());
    &text[..end]
}

fn is_tool_clause_boundary(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r' | '。' | '！' | '？' | '!' | '?' | '；' | ';' | '：' | ':'
    )
}

fn contains_ascii_term(text: &str, term: &str) -> bool {
    text.match_indices(term).any(|(index, _)| {
        let before = text[..index].chars().next_back();
        let after = text[index + term.len()..].chars().next();
        let is_term_character =
            |character: char| character.is_ascii_alphanumeric() || character == '_';
        !before.is_some_and(is_term_character) && !after.is_some_and(is_term_character)
    })
}

fn english_clause_promises_tool_work(clause: &str) -> bool {
    if [
        "will not",
        "won't",
        "do not",
        "don't",
        "no need to",
        "without running",
    ]
    .iter()
    .any(|negation| contains_ascii_term(clause, negation))
    {
        return false;
    }

    let explicit_tool = [
        "use the shell",
        "use shell",
        "use rg",
        "get-content",
        "select-string",
    ]
    .iter()
    .any(|tool| contains_ascii_term(clause, tool));
    let action = [
        "check", "inspect", "read", "search", "run", "verify", "open", "locate",
    ]
    .iter()
    .any(|action| contains_ascii_term(clause, action));
    let target = [
        "file",
        "files",
        "repository",
        "repo",
        "project",
        "code",
        "test",
        "tests",
        "command",
        "commands",
        "shell",
        "log",
        "logs",
        "path",
        "config",
        "configuration",
        "error",
        "issue",
        "output",
        "workspace",
        "it",
        "this",
    ]
    .iter()
    .any(|target| contains_ascii_term(clause, target));
    explicit_tool || (action && target)
}

fn chinese_clause_promises_tool_work(clause: &str, action_markers: &[&str]) -> bool {
    if [
        "不需要",
        "无需",
        "不用",
        "不必",
        "不会",
        "不再",
        "不要",
        "先不",
        "暂不",
    ]
    .iter()
    .any(|negation| clause.contains(negation))
    {
        return false;
    }

    let explicit_tool = [
        "执行命令",
        "运行命令",
        "打开终端",
        "用 shell",
        "使用 shell",
        "用 rg",
        "使用 rg",
        "get-content",
        "select-string",
    ]
    .iter()
    .any(|tool| clause.to_ascii_lowercase().contains(tool));
    let action = action_markers.iter().any(|action| clause.contains(action))
        || ["查一下", "看一下"]
            .iter()
            .any(|action| clause.contains(action));
    let target = [
        "文件",
        "代码",
        "项目",
        "仓库",
        "目录",
        "路径",
        "日志",
        "脚本",
        "命令",
        "终端",
        "测试",
        "配置",
        "调用链",
        "错误",
        "报错",
        "问题",
        "结果",
        "位置",
        "实现",
        "依赖",
        "工作区",
        "迁移",
        "schema",
        "字段",
        "帮助",
    ]
    .iter()
    .any(|target| clause.contains(target));
    explicit_tool || (action && target)
}

fn strip_leading_acknowledgement(text: &str) -> &str {
    const PREFIXES: [&str; 12] = [
        "好的。",
        "好的，",
        "好的,",
        "可以。",
        "可以，",
        "明白。",
        "明白，",
        "收到。",
        "收到，",
        "没问题。",
        "没问题，",
        "好。",
    ];
    PREFIXES
        .iter()
        .find_map(|prefix| text.strip_prefix(prefix).map(str::trim_start))
        .unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stop_turn(text: &str, opaque: serde_json::Value) -> ModelTurn {
        ModelTurn {
            text: text.to_string(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            usage: None,
            terminal: ProviderTerminal::Stop,
            opaque,
        }
    }

    #[test]
    fn detects_tool_intent_without_tool_call() {
        assert!(promised_tool_work_without_call(
            "先看一下当前文件中用到这些字符的位置："
        ));
        assert!(promised_tool_work_without_call(
            "I'll inspect the file with rg first."
        ));
        assert!(promised_tool_work_without_call(
            "我会先查看当前文件，再运行针对性测试。"
        ));
        assert!(promised_tool_work_without_call(
            "好的。我先了解 v2.2 的迁移方式与 IR 2.2 新字段（面部表演、对话耦合），再执行升级并加台词。先查看迁移帮助和 schema："
        ));
        assert!(!promised_tool_work_without_call(
            "检查结果：语法检查通过，程序启动成功。"
        ));
        assert!(!promised_tool_work_without_call(
            "先看结论：问题来自配置，不需要调用工具。"
        ));
        assert!(!promised_tool_work_without_call(
            "I will not run commands; I can answer directly."
        ));
    }

    #[test]
    fn normal_answer_without_tools_does_not_continue() {
        let mut turn = stop_turn("The answer is 42.", json!({}));
        apply_turn_continuation(ApiFamily::OpenAi, true, &mut turn);
        assert_eq!(turn.terminal, ProviderTerminal::Stop);
    }

    #[test]
    fn incomplete_no_tool_turn_continues_within_limit() {
        let mut turn = stop_turn("I'll inspect the file with rg first.", json!({}));
        apply_turn_continuation(ApiFamily::OpenAi, true, &mut turn);
        assert_eq!(
            turn.terminal,
            ProviderTerminal::Continue(ContinuationReason::Incomplete)
        );
    }

    #[test]
    fn incomplete_no_tool_turn_does_not_continue_without_tools() {
        let mut turn = stop_turn("I'll inspect the file with rg first.", json!({}));
        apply_turn_continuation(ApiFamily::OpenAi, false, &mut turn);
        assert_eq!(turn.terminal, ProviderTerminal::Stop);
    }

    #[test]
    fn deepseek_continues_when_visible_text_promises_tool_work() {
        let mut turn = stop_turn(
            "好的。我先了解 v2.2 的迁移方式与 IR 2.2 新字段，再执行升级。",
            json!({}),
        );
        apply_turn_continuation(ApiFamily::DeepSeek, true, &mut turn);
        assert_eq!(
            turn.terminal,
            ProviderTerminal::Continue(ContinuationReason::Incomplete)
        );

        let mut completed = stop_turn("升级完成，所有验证均已通过。", json!({}));
        apply_turn_continuation(ApiFamily::DeepSeek, true, &mut completed);
        assert_eq!(completed.terminal, ProviderTerminal::Stop);
    }

    #[test]
    fn codex_non_terminal_end_turn_requests_continuation() {
        let mut turn = stop_turn("", json!({ "end_turn": false }));
        apply_turn_continuation(ApiFamily::Codex, true, &mut turn);
        assert_eq!(
            turn.terminal,
            ProviderTerminal::Continue(ContinuationReason::ProviderRequested)
        );
    }

    #[test]
    fn codex_terminal_end_turn_stops() {
        let mut turn = stop_turn("done", json!({ "end_turn": true }));
        apply_turn_continuation(ApiFamily::Codex, true, &mut turn);
        assert_eq!(turn.terminal, ProviderTerminal::Stop);
    }
}
