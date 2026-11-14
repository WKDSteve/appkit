//! A call of the process helper (`appkit_core::child`, which needs a tokio runtime) from code
//! that blocks: the future runs on a thread of its own with a runtime of its own, and the caller
//! waits for it. So the caller may itself be inside a runtime, or outside of one.

use std::future::Future;

/// Run the future `make` gives to its end and return what it came to.
///
/// # Errors
///
/// No runtime or thread could be made, or the call panicked.
pub(crate) fn block_on<F, T>(make: impl FnOnce() -> F + Send) -> Result<T, String>
where
    F: Future<Output = T>,
    T: Send,
{
    let call = || -> Result<T, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("no runtime for the call: {e}"))?;
        Ok(runtime.block_on(make()))
    };
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("child-call".to_owned())
            .spawn_scoped(scope, call)
            .map_err(|e| format!("no thread for the call: {e}"))?
            .join()
            .map_err(|_| "the call broke off".to_owned())?
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_future_runs_to_its_end_outside_of_a_runtime_and_inside_of_one() {
        assert_eq!(block_on(|| async { 7 }), Ok(7));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let inside = runtime.block_on(async {
            block_on(|| async {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                8
            })
        });
        assert_eq!(inside, Ok(8));
    }

    #[test]
    fn a_call_that_panics_is_an_error() {
        let broke: Result<(), String> = block_on(|| async { panic!("on purpose") });
        assert_eq!(broke, Err("the call broke off".to_owned()));
    }
}
