//! A hidden window is revealed only after the page acknowledges its final layout.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowRequest {
    pub request_id: u64,
    pub mode: String,
}

#[derive(Default)]
pub struct Presentation {
    pub ready: bool,
    pub closing: bool,
    next: u64,
    pub pending: Option<ShowRequest>,
}
impl Presentation {
    pub fn request(&mut self, mode: &str) -> ShowRequest {
        self.closing = false;
        self.next += 1;
        let request = ShowRequest { request_id: self.next, mode: mode.into() };
        self.pending = Some(request.clone());
        request
    }
    pub fn acknowledge(&mut self, id: u64) -> bool {
        if !self.ready || self.pending.as_ref().is_none_or(|p| p.request_id != id) {
            return false;
        }
        self.pending = None;
        true
    }
    pub fn cancel(&mut self) {
        self.pending = None;
        self.closing = true;
    }
    pub fn is_current(&self, id: u64) -> bool {
        self.next == id
    }
    /// A click in another app is meaningful even if this window never gained
    /// focus. Pending layouts, hidden windows and an existing close are ignored.
    pub fn outside_click_request(&self, visible: bool) -> Option<u64> {
        (visible && self.next > 0 && self.pending.is_none() && !self.closing).then_some(self.next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outside_click_dismisses_presented_window_without_needing_focus() {
        let mut p = Presentation { ready: true, ..Default::default() };
        assert_eq!(p.outside_click_request(true), None);
        let first = p.request("command");
        assert_eq!(p.outside_click_request(true), None);
        assert!(p.acknowledge(first.request_id));
        assert_eq!(p.outside_click_request(false), None);
        assert_eq!(p.outside_click_request(true), Some(first.request_id));
        p.cancel();
        assert_eq!(p.outside_click_request(true), None);
        let next = p.request("command");
        assert_eq!(p.outside_click_request(true), None);
        assert!(p.acknowledge(next.request_id));
        assert_eq!(p.outside_click_request(true), Some(next.request_id));
        assert!(!p.is_current(first.request_id));
    }
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
    #[test]
    fn a_delayed_close_cannot_hide_a_newer_open() {
        let mut p = Presentation { ready: true, ..Default::default() };
        let old = p.request("command");
        assert!(p.acknowledge(old.request_id));
        p.cancel();
        assert!(p.closing);
        let new = p.request("command");
        assert!(!p.closing);
        assert!(!p.is_current(old.request_id));
        assert!(p.is_current(new.request_id));
        assert!(p.acknowledge(new.request_id));
        assert!(!p.is_current(old.request_id));
    }
}
