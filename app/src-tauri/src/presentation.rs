//! A hidden window is revealed only after the page acknowledges its final layout.
#[derive(Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowRequest {
    pub request_id: u64,
    pub mode: String,
}

/// Where the window is in its show/close cycle.
#[derive(Clone, Default, PartialEq)]
pub enum Phase {
    #[default]
    Hidden,
    /// Shown on request; revealed once the page acknowledges this layout.
    AwaitingLayout(ShowRequest),
    Visible,
    /// The page is playing its close animation and will call back to hide.
    Closing,
}

#[derive(Default)]
pub struct Presentation {
    /// The page has loaded and can answer show requests.
    pub ready: bool,
    phase: Phase,
    next: u64,
}
impl Presentation {
    pub fn request(&mut self, mode: &str) -> ShowRequest {
        self.next += 1;
        let request = ShowRequest { request_id: self.next, mode: mode.into() };
        self.phase = Phase::AwaitingLayout(request.clone());
        request
    }
    pub fn acknowledge(&mut self, id: u64) -> bool {
        match &self.phase {
            Phase::AwaitingLayout(p) if self.ready && p.request_id == id => {
                self.phase = Phase::Visible;
                true
            }
            _ => false,
        }
    }
    /// Starts closing: the window fades out, then `hidden` is called.
    pub fn cancel(&mut self) {
        self.phase = Phase::Closing;
    }
    /// The window is off screen.
    pub fn hidden(&mut self) {
        self.phase = Phase::Hidden;
    }
    pub fn pending(&self) -> Option<&ShowRequest> {
        match &self.phase {
            Phase::AwaitingLayout(p) => Some(p),
            _ => None,
        }
    }
    pub fn closing(&self) -> bool {
        self.phase == Phase::Closing
    }
    pub fn is_current(&self, id: u64) -> bool {
        self.next == id
    }
    /// A click in another app is meaningful even if this window never gained
    /// focus. Pending layouts, hidden windows and an existing close are ignored.
    pub fn outside_click_request(&self, visible: bool) -> Option<u64> {
        (visible && self.phase == Phase::Visible).then_some(self.next)
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
        assert_eq!(p.pending().unwrap().mode, "welcome");
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
        assert!(p.closing());
        let new = p.request("command");
        assert!(!p.closing());
        assert!(!p.is_current(old.request_id));
        assert!(p.is_current(new.request_id));
        assert!(p.acknowledge(new.request_id));
        assert!(!p.is_current(old.request_id));
    }
    #[test]
    fn hiding_ends_the_close() {
        let mut p = Presentation { ready: true, ..Default::default() };
        let shown = p.request("command");
        assert!(p.acknowledge(shown.request_id));
        p.cancel();
        p.hidden();
        assert!(!p.closing() && p.pending().is_none() && p.outside_click_request(true).is_none());
    }
}
