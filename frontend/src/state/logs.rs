use common::LOG_COLLECTOR;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level { Debug, Info, Warn, Error }

/// 日志面板的级别筛选；“信息”含 INFO 与 DEBUG
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelFilter { All, Info, Warn, Error }

#[derive(Clone, Debug)]
pub struct LogLine {
    /// HH:MM:SS
    pub time: String,
    pub level: Level,
    /// 原始任务标签，例如 “#3 张三”
    pub label: Option<String>,
    /// 单行显示用，换行已替换为空格
    pub msg: String,
    pub raw: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogFilter {
    pub level: LevelFilter,
    pub label: Option<String>,
    pub search: String,
    pub follow: bool,
}

impl Default for LogFilter {
    fn default() -> Self {
        Self { level: LevelFilter::All, label: None, search: String::new(), follow: true }
    }
}

#[derive(Default)]
pub struct LogStore {
    pub lines: Vec<LogLine>,
    /// 出现过的任务标签，按首次出现顺序
    pub labels: Vec<String>,
    /// 当前筛选结果在 lines 中的下标
    pub filtered: Vec<usize>,
    /// 截断或清空时递增，使筛选缓存失效
    generation: u64,
    cache_key: Option<(usize, u64, LogFilter)>,
}

const MAX_LINES: usize = 5000;

impl LogStore {
    /// 收取日志收集器里的新日志，返回其中 ERROR 级的行（用于弹消息条）
    pub fn ingest(&mut self) -> Vec<LogLine> {
        let Some(raws) = LOG_COLLECTOR.lock().unwrap().get_logs() else {
            return Vec::new();
        };
        let mut errors = Vec::new();
        for raw in raws {
            let line = parse(raw);
            if let Some(label) = &line.label {
                if !self.labels.contains(label) {
                    self.labels.push(label.clone());
                }
            }
            if line.level == Level::Error {
                errors.push(line.clone());
            }
            self.lines.push(line);
        }
        while self.lines.len() > MAX_LINES {
            self.lines.drain(0..MAX_LINES / 2);
            self.generation += 1;
        }
        errors
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.filtered.clear();
        self.generation += 1;
    }

    /// 条数、级别、任务、搜索词任一变化时重算筛选结果
    pub fn refilter(&mut self, filter: &LogFilter) {
        let key = (self.lines.len(), self.generation, LogFilter { follow: false, ..filter.clone() });
        if self.cache_key.as_ref() == Some(&key) {
            return;
        }
        let search = filter.search.to_lowercase();
        self.filtered = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| match filter.level {
                LevelFilter::All => true,
                LevelFilter::Info => matches!(l.level, Level::Info | Level::Debug),
                LevelFilter::Warn => l.level == Level::Warn,
                LevelFilter::Error => l.level == Level::Error,
            })
            .filter(|(_, l)| filter.label.is_none() || l.label == filter.label)
            .filter(|(_, l)| search.is_empty() || l.msg.to_lowercase().contains(&search))
            .map(|(i, _)| i)
            .collect();
        self.cache_key = Some(key);
    }
}

/// “#3 张三” → “No.0003 张三”
pub fn display_label(label: &str) -> String {
    let Some(rest) = label.strip_prefix('#') else {
        return label.to_owned();
    };
    let (seq, name) = rest.split_once(' ').unwrap_or((rest, ""));
    match seq.parse::<u64>() {
        Ok(n) if name.is_empty() => format!("No.{n:04}"),
        Ok(n) => format!("No.{n:04} {name}"),
        Err(_) => label.to_owned(),
    }
}

/// 解析 `[%Y-%m-%d %H:%M:%S:%3f] LEVEL [label]: msg` 或 `[..] LEVEL: msg`（见 common/record_log.rs）
fn parse(raw: String) -> LogLine {
    let fallback = |raw: String| LogLine {
        time: String::new(),
        level: Level::Info,
        label: None,
        msg: raw.replace('\n', " "),
        raw,
    };
    let Some(rest) = raw.strip_prefix('[') else { return fallback(raw.clone()) };
    let Some((stamp, rest)) = rest.split_once("] ") else { return fallback(raw.clone()) };
    let level_end = rest.find(|c: char| c == ' ' || c == ':').unwrap_or(rest.len());
    let level = match &rest[..level_end] {
        "ERROR" => Level::Error,
        "WARN" => Level::Warn,
        "INFO" => Level::Info,
        "DEBUG" | "TRACE" => Level::Debug,
        _ => return fallback(raw.clone()),
    };
    let rest = &rest[level_end..];
    let (label, msg) = if let Some(tail) = rest.strip_prefix(" [") {
        match tail.split_once("]: ") {
            Some((label, msg)) => (Some(label.to_owned()), msg),
            None => (None, tail),
        }
    } else {
        (None, rest.strip_prefix(": ").unwrap_or(rest))
    };
    LogLine {
        time: stamp.get(11..19).unwrap_or("").to_owned(),
        level,
        label,
        msg: msg.replace('\n', " "),
        raw: raw.clone(),
    }
}
