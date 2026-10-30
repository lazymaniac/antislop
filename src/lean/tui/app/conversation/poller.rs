use super::*;

pub(super) struct Poller {
    pub(super) client: Client,
    pub(super) updates: std::sync::mpsc::Sender<Update>,
    pub(super) conversation: String,
    pub(super) generation: u64,
    pub(super) stop: Arc<AtomicBool>,
}

impl Poller {
    pub(super) fn spawn(self) {
        std::thread::spawn(move || self.run());
    }

    fn run(self) {
        let mut after = 0;
        let mut wait_ms = 0;
        while !self.stop.load(Ordering::SeqCst) {
            if !self.poll_once(&mut after, &mut wait_ms) {
                break;
            }
        }
    }

    fn poll_once(&self, after: &mut u64, wait_ms: &mut u64) -> bool {
        let result = self.client.call(&Request::Events {
            conversation: self.conversation.clone(),
            after: *after,
            wait_ms: *wait_ms,
        });
        let update = self.make_update(result, after, wait_ms);
        !self.stop.load(Ordering::SeqCst) && self.updates.send(update).is_ok()
    }

    fn make_update(&self, result: Result<Value>, after: &mut u64, wait_ms: &mut u64) -> Update {
        match result {
            Ok(reply) => self.event_update(reply, after, wait_ms),
            Err(error) => {
                std::thread::sleep(Duration::from_secs(1));
                Update::Notice(format!("daemon is not responding: {error}"))
            }
        }
    }

    fn event_update(&self, reply: Value, after: &mut u64, wait_ms: &mut u64) -> Update {
        let events = reply["events"].as_array().cloned().unwrap_or_default();
        *wait_ms = if events.is_empty() { 10_000 } else { 0 };
        if let Some(last) = events.last() {
            *after = last["seq"].as_u64().unwrap_or(*after);
        }
        Update::Events {
            conversation: self.conversation.clone(),
            generation: self.generation,
            events,
            idle: reply["idle"] == true,
            autonomous: reply["autonomous"] == true,
        }
    }
}
