//! Calls macOS answers only on the main thread, made from any thread.
//!
//! HIToolbox requires the Text Input Sources functions to run on the main dispatch queue and stops the process with
//! SIGILL when one runs elsewhere (`dispatch_assert_queue` under `TISCopyCurrentKeyboardInputSource`). KeySwitch asks
//! for the current layout from the engine's worker every half second and from the event tap's thread on every key: on a
//! MacBook Pro with macOS 13.7.8 every start ended that way within seconds.
//!
//! `MainThreadRunner` hands such a call to the main thread and waits for it with a time limit. A call the main thread
//! does not begin in time is withdrawn, so it never runs late behind the caller's back, and the caller keeps its
//! fallback. The module is plain Rust: `macos_native` supplies the two system pieces, whether the current thread is the
//! main one and how a job number reaches the main queue.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct JobState {
    started: bool,
    done: bool,
}

struct Job {
    function: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    state: Mutex<JobState>,
    finished: Condvar,
}

/// Run a function on the main thread and wait for its answer, within a time limit.
pub struct MainThreadRunner {
    is_main_thread: fn() -> bool,
    post: fn(u64),
    jobs: Mutex<Vec<(u64, Arc<Job>)>>,
    numbers: AtomicU64,
}

impl MainThreadRunner {
    pub const fn new(is_main_thread: fn() -> bool, post: fn(u64)) -> Self {
        MainThreadRunner { is_main_thread, post, jobs: Mutex::new(Vec::new()), numbers: AtomicU64::new(1) }
    }

    /// `function()` on the main thread, or `fallback` if the main thread does not begin it in time.
    pub fn call<T: Send + 'static>(&self, function: impl FnOnce() -> T + Send + 'static, fallback: T, timeout: f64) -> T {
        if (self.is_main_thread)() {
            return function();
        }
        let answer: Arc<Mutex<Option<T>>> = Arc::new(Mutex::new(None));
        let slot = answer.clone();
        let job = Arc::new(Job {
            function: Mutex::new(Some(Box::new(move || *lock(&slot) = Some(function())))),
            state: Mutex::new(JobState::default()),
            finished: Condvar::new(),
        });
        let number = self.numbers.fetch_add(1, Ordering::Relaxed);
        lock(&self.jobs).push((number, job.clone()));
        (self.post)(number);
        let state = lock(&job.state);
        let (state, _) = job
            .finished
            .wait_timeout_while(state, Duration::from_secs_f64(timeout), |state| !state.done)
            .unwrap_or_else(PoisonError::into_inner);
        if !state.done {
            drop(state);
            // The jobs lock first and the job's state under it, in the order `run` takes them.
            let mut jobs = lock(&self.jobs);
            let started = lock(&job.state).started;
            if !started {
                jobs.retain(|(known, _)| *known != number);
                return fallback;
            }
            drop(jobs);
            // The main thread began it just now; the system call it makes is short.
            let state = lock(&job.state);
            drop(job.finished.wait_while(state, |state| !state.done).unwrap_or_else(PoisonError::into_inner));
        } else {
            drop(state);
        }
        let result = lock(&answer).take();
        result.unwrap_or(fallback)
    }

    /// Run the posted job `number`. Called on the main thread; a withdrawn job is skipped.
    pub fn run(&self, number: u64) {
        let job = {
            let mut jobs = lock(&self.jobs);
            let Some(index) = jobs.iter().position(|(known, _)| *known == number) else {
                return;
            };
            let (_, job) = jobs.remove(index);
            lock(&job.state).started = true;
            job
        };
        let function = lock(&job.function).take();
        if let Some(function) = function {
            function();
        }
        lock(&job.state).done = true;
        job.finished.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::thread;

    static ON_MAIN: AtomicBool = AtomicBool::new(false);
    static RUNNER: MainThreadRunner = MainThreadRunner::new(|| ON_MAIN.load(Ordering::SeqCst), |_| {});
    static POSTING: MainThreadRunner = MainThreadRunner::new(|| false, |_| {});

    #[test]
    fn the_main_thread_calls_directly() {
        ON_MAIN.store(true, Ordering::SeqCst);
        assert_eq!(RUNNER.call(|| 7, 0, 0.1), 7);
        ON_MAIN.store(false, Ordering::SeqCst);
    }

    #[test]
    fn a_call_the_main_thread_never_begins_is_withdrawn_and_never_runs_late() {
        static RAN: AtomicBool = AtomicBool::new(false);
        let answer = POSTING.call(|| { RAN.store(true, Ordering::SeqCst); 1 }, 9, 0.05);
        assert_eq!(answer, 9);
        // The job is gone: running its number now does nothing.
        POSTING.run(POSTING.numbers.load(Ordering::Relaxed) - 1);
        assert!(!RAN.load(Ordering::SeqCst));
    }

    #[test]
    fn a_job_run_by_the_main_thread_answers_the_caller() {
        static SERVED: MainThreadRunner = MainThreadRunner::new(|| false, |number| {
            thread::spawn(move || SERVED.run(number));
        });
        assert_eq!(SERVED.call(|| "text".to_owned(), String::new(), 1.0), "text");
    }
}
