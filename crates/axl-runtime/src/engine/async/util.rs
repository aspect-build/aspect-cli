use starlark::{
    environment::GlobalsBuilder, eval::Evaluator, starlark_module, values::tuple::UnpackTuple,
};

use crate::engine::store::Env;

use super::{future::StarlarkFuture, future_stream::FutureIterator};

pub fn register_globals(globals: &mut GlobalsBuilder) {
    globals.namespace("futures", register_future_utils);
}

#[starlark_module]
fn register_future_utils(globals: &mut GlobalsBuilder) {
    fn iter<'v>(
        #[starlark(args)] futures: UnpackTuple<StarlarkFuture>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<FutureIterator> {
        let env = Env::from_eval(eval)?;
        return Ok(FutureIterator::new(env.rt.clone(), futures.items));
    }
}
