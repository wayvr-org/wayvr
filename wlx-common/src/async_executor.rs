use std::rc::Rc;

pub type AsyncExecutor = Rc<smol::LocalExecutor<'static>>;

pub fn create_local() -> AsyncExecutor {
	Rc::new(smol::LocalExecutor::new())
}
