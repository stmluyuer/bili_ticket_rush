pub mod draft;
pub mod jobs;
pub mod logs;
pub mod persist;
pub mod remote;
pub mod tasks;

#[derive(Clone, Debug)]
pub struct BuyerList {
    pub list: Vec<common::ticket::BuyerInfo>,
    pub from_cache: bool,
}
