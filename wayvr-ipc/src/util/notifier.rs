use async_channel::{Receiver, Sender};

// Copyable wrapped Notify struct for easier usage
#[derive(Clone)]
pub struct Notifier {
	sender: Sender<()>,
	receiver: Receiver<()>,
}

impl Notifier {
	pub fn new() -> Self {
		let (sender, receiver) = async_channel::bounded(1);
		Self { sender, receiver }
	}

	pub fn notify(&self) {
		let _ = self.sender.try_send(());
	}

	pub async fn wait(&self) {
		let _ = self.receiver.recv().await;
	}
}

impl Default for Notifier {
	fn default() -> Self {
		Self::new()
	}
}
