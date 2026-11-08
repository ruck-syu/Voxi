//! A hidden window is revealed only after the page acknowledges its final layout.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowRequest { pub request_id: u64, pub mode: String }

#[derive(Default)]
pub struct Presentation {
    pub ready: bool,
    next: u64,
    pub pending: Option<ShowRequest>,
}
impl Presentation {
    pub fn request(&mut self, mode: &str) -> ShowRequest {
        self.next += 1;
        let request = ShowRequest { request_id: self.next, mode: mode.into() };
        self.pending = Some(request.clone());
        request
    }
    pub fn acknowledge(&mut self, id: u64) -> bool {
        if !self.ready || !self.pending.as_ref().is_some_and(|p| p.request_id == id) { return false; }
        self.pending = None;
        true
    }
    pub fn cancel(&mut self) { self.pending = None; }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn early_open_waits_for_ready_and_the_matching_layout() {
        let mut p = Presentation::default();
        let request = p.request("welcome");
        assert!(!p.acknowledge(request.request_id));
        p.ready = true;
        assert_eq!(p.pending.as_ref().unwrap().mode, "welcome");
        assert!(p.acknowledge(request.request_id));
        assert!(!p.acknowledge(request.request_id));
    }
    #[test]
    fn stale_or_cancelled_layouts_cannot_reopen_the_window() {
        let mut p = Presentation { ready: true, ..Default::default() };
        let old = p.request("settings");
        let new = p.request("command");
        assert!(!p.acknowledge(old.request_id));
        p.cancel();
        assert!(!p.acknowledge(new.request_id));
    }
}
