use common::ticket::TicketInfo;
use super::remote::Remote;

#[derive(Clone, Debug, PartialEq)]
pub enum ParseResult {
    Empty,
    Id { id: String, referer: String },
    ShortLink,
    Invalid,
}

pub fn parse_project_input(input: &str) -> ParseResult {
    if input.is_empty() { return ParseResult::Empty; }
    if input.contains("b23.tv") { return ParseResult::ShortLink; }
    if input.contains("https") {
        if let Some(start) = input.find("id=") {
            let id = input[start + 3..].split('&').next().unwrap();
            if (5..=8).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit()) {
                return ParseResult::Id { id: id.to_owned(), referer: input.to_owned() };
            }
        }
        return ParseResult::Invalid;
    }
    if input.bytes().all(|b| b.is_ascii_digit()) {
        return ParseResult::Id {
            id: input.to_owned(),
            referer: format!("https://show.bilibili.com/platform/detail.html?id={input}"),
        };
    }
    ParseResult::Invalid
}

pub struct TaskDraft {
    pub account_uid: Option<i64>,
    pub project_input: String,
    pub parsed: ParseResult,
    pub project: Remote<TicketInfo>,
    pub project_for: Option<(i64, String)>,
    pub mode: u8,
    pub screen_idx: Option<usize>,
    pub ticket_id: Option<i64>,
    pub selected_buyers: Vec<i64>,
    pub count: i32,
    pub contact_name: String,
    pub contact_tel: String,
    pub skip_input: String,
    pub skip_words: Vec<String>,
    pub select_after_refresh: Option<String>,
    pub last_paste_len: usize,
}

impl Default for TaskDraft {
    fn default() -> Self {
        Self {
            account_uid: None, project_input: String::new(), parsed: ParseResult::Empty,
            project: Remote::Idle, project_for: None, mode: 0, screen_idx: None,
            ticket_id: None, selected_buyers: Vec::new(), count: 1,
            contact_name: String::new(), contact_tel: String::new(), skip_input: String::new(),
            skip_words: Vec::new(), select_after_refresh: None, last_paste_len: 0,
        }
    }
}
