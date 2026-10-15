use coterm_proto::{Actor, CommandItem};
use regex::Regex;

#[derive(Debug, PartialEq, Eq)]
pub enum ParserState {
    Prompt,
    Command,
    Output,
    Idle,
}

pub struct Osc133Parser {
    state: ParserState,
    command_counter: u64,
    current_item: Option<CommandItem>,
    completed_items: Vec<CommandItem>,
    osc133_regex: Regex,
}

impl Default for Osc133Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Osc133Parser {
    pub fn new() -> Self {
        // 匹配 \x1b]133;[A-D](;.*)?(\x07|\x1b\\)
        let osc133_regex = Regex::new(r"\x1b\]133;([A-D])(?:;([^\x07\x1b]*))?(?:\x07|\x1b\\)").unwrap();
        Self {
            state: ParserState::Idle,
            command_counter: 0,
            current_item: None,
            completed_items: Vec::new(),
            osc133_regex,
        }
    }

    /// 标记新的一行命令开始执行
    pub fn start_command(&mut self, command: String, actor: Actor, cwd: Option<String>) -> String {
        self.command_counter += 1;
        let item = CommandItem::new(self.command_counter, command, actor, cwd);
        let item_id = item.id.clone();
        self.current_item = Some(item);
        self.state = ParserState::Command;
        item_id
    }

    /// 接收流式输出，解析 OSC 133 标签并累积命令输出
    pub fn feed_output(&mut self, chunk: &str) {
        if let Some(caps) = self.osc133_regex.captures(chunk) {
            let code = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            match code {
                "A" => self.state = ParserState::Prompt,
                "B" => self.state = ParserState::Command,
                "C" => self.state = ParserState::Output,
                "D" => {
                    // 命令结束
                    let exit_code = caps.get(2)
                        .and_then(|m| m.as_str().parse::<i32>().ok())
                        .unwrap_or(0);
                    
                    if let Some(mut item) = self.current_item.take() {
                        item.exit_code = Some(exit_code);
                        self.completed_items.push(item);
                    }
                    self.state = ParserState::Idle;
                }
                _ => {}
            }
        }

        // 如果当前处于 Output 状态且有未完成项，累加输出
        if let Some(ref mut item) = self.current_item {
            // 简单过滤掉 OSC 133 控制符
            let clean_chunk = self.osc133_regex.replace_all(chunk, "");
            item.output.push_str(&clean_chunk);
        }
    }

    pub fn completed_items(&self) -> &[CommandItem] {
        &self.completed_items
    }

    pub fn get_last_failed(&self) -> Option<&CommandItem> {
        self.completed_items
            .iter()
            .rev()
            .find(|item| item.exit_code.map(|c| c != 0).unwrap_or(false))
    }
}
