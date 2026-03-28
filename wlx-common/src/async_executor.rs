use std::rc::Rc;

pub type AsyncExecutor = Rc<smol::LocalExecutor<'static>>;
