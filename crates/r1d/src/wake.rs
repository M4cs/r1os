//! Waking the R1 for a trigger that's due (ADR-0024). With the screen off Android suspends the CPU, and
//! r1d's timers stop with it. This sets one kernel alarm for the next due time, the way Android's own
//! AlarmManager does: a `timerfd` on `CLOCK_REALTIME_ALARM` (it wakes the system from suspend; root has
//! `CAP_WAKE_ALARM`), watched by `epoll` with `EPOLLWAKEUP` (the kernel holds the system awake from the alarm
//! until we wait again; root has `CAP_BLOCK_SUSPEND`). So the R1 stays awake exactly until the scheduler says
//! the due triggers ran ([`Alarm::done`]), then may sleep again.
//!
//! Nothing is armed when nothing is due. Off Android there's no alarm: r1d's own timers are enough.

use std::sync::{Arc, Condvar, Mutex};
#[cfg(target_os = "android")]
use std::time::Duration;

/// How long a woken R1 waits for the scheduler before it may sleep again anyway.
#[cfg(target_os = "android")]
const MAX_AWAKE: Duration = Duration::from_secs(30);

pub struct Alarm {
    /// The wall-clock time (ms) to wake at, and whether it changed since the thread last armed it.
    target: Mutex<(Option<i64>, bool)>,
    /// The alarm fired and the scheduler hasn't said it's done: (fired, done).
    state: Mutex<(bool, bool)>,
    cond: Condvar,
    /// Tells the scheduler the alarm fired.
    pub fired: tokio::sync::Notify,
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    kick: Mutex<Option<i32>>,
}

impl Alarm {
    /// Starts the alarm thread (Android), or a no-op alarm.
    pub fn start() -> Arc<Alarm> {
        let alarm = Arc::new(Alarm {
            target: Mutex::new((None, false)),
            state: Mutex::new((false, false)),
            cond: Condvar::new(),
            fired: tokio::sync::Notify::new(),
            kick: Mutex::new(None),
        });
        #[cfg(target_os = "android")]
        {
            let a = alarm.clone();
            if let Err(e) = std::thread::Builder::new().name("r1d-wake".into()).spawn(move || a.run()) {
                tracing::warn!(error = %e, "wake alarm: no thread; triggers won't wake the R1");
            }
        }
        alarm
    }

    /// Wake at `at_ms` (wall clock), or never (`None`).
    pub fn set(&self, at_ms: Option<i64>) {
        {
            let mut t = self.target.lock().unwrap();
            if t.0 == at_ms {
                return;
            }
            *t = (at_ms, true);
        }
        #[cfg(target_os = "android")]
        if let Some(fd) = *self.kick.lock().unwrap() {
            let one: u64 = 1;
            // SAFETY: writing 8 bytes from a live u64 to our eventfd.
            unsafe { libc::write(fd, (&raw const one).cast(), 8) };
        }
    }

    /// The scheduler ran what the alarm woke it for: the R1 may sleep again.
    pub fn done(&self) {
        let mut s = self.state.lock().unwrap();
        if s.0 {
            s.1 = true;
            self.cond.notify_all();
        }
    }

    #[cfg(target_os = "android")]
    fn run(self: Arc<Self>) {
        // SAFETY: plain syscalls on file descriptors this thread owns; the buffers are live locals.
        unsafe {
            let tfd = libc::timerfd_create(libc::CLOCK_REALTIME_ALARM, libc::TFD_CLOEXEC);
            let kick = libc::eventfd(0, libc::EFD_CLOEXEC);
            let ep = libc::epoll_create1(libc::EPOLL_CLOEXEC);
            if tfd < 0 || kick < 0 || ep < 0 {
                tracing::warn!(error = %std::io::Error::last_os_error(), "wake alarm: unavailable; triggers won't wake the R1");
                return;
            }
            let mut ev = libc::epoll_event { events: (libc::EPOLLIN | libc::EPOLLWAKEUP) as u32, u64: 1 };
            if libc::epoll_ctl(ep, libc::EPOLL_CTL_ADD, tfd, &raw mut ev) < 0 {
                tracing::warn!(error = %std::io::Error::last_os_error(), "wake alarm: no EPOLLWAKEUP");
                return;
            }
            let mut ev = libc::epoll_event { events: libc::EPOLLIN as u32, u64: 2 };
            libc::epoll_ctl(ep, libc::EPOLL_CTL_ADD, kick, &raw mut ev);
            *self.kick.lock().unwrap() = Some(kick);
            tracing::info!("wake alarm ready");
            let mut events = [libc::epoll_event { events: 0, u64: 0 }; 2];
            loop {
                self.arm(tfd);
                let n = libc::epoll_wait(ep, events.as_mut_ptr(), 2, -1);
                if n < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    tracing::warn!(error = %std::io::Error::last_os_error(), "wake alarm: epoll failed");
                    return;
                }
                let mut buf = 0u64;
                for e in &events[..n as usize] {
                    if e.u64 == 2 {
                        libc::read(kick, (&raw mut buf).cast(), 8);
                    } else if e.u64 == 1 && libc::read(tfd, (&raw mut buf).cast(), 8) == 8 {
                        // Awake (EPOLLWAKEUP holds it until the next epoll_wait): let the scheduler run.
                        self.target.lock().unwrap().0 = None;
                        tracing::info!("wake alarm fired");
                        self.awake();
                    }
                }
            }
        }
    }

    /// Arms the timer for the target, if it changed.
    #[cfg(target_os = "android")]
    fn arm(&self, tfd: i32) {
        let (target, changed) = {
            let mut t = self.target.lock().unwrap();
            let changed = std::mem::replace(&mut t.1, false);
            (t.0, changed)
        };
        if !changed {
            return;
        }
        let value = match target {
            Some(ms) => libc::timespec { tv_sec: (ms / 1000) as libc::time_t, tv_nsec: ((ms % 1000) * 1_000_000) as libc::c_long },
            None => libc::timespec { tv_sec: 0, tv_nsec: 0 }, // disarmed
        };
        let spec = libc::itimerspec { it_interval: libc::timespec { tv_sec: 0, tv_nsec: 0 }, it_value: value };
        // SAFETY: tfd is our timerfd; spec is a live local.
        if unsafe { libc::timerfd_settime(tfd, libc::TFD_TIMER_ABSTIME, &raw const spec, std::ptr::null_mut()) } < 0 {
            tracing::warn!(error = %std::io::Error::last_os_error(), "wake alarm: couldn't arm");
        }
    }

    /// Tells the scheduler, and stays awake until it's done (or [`MAX_AWAKE`]).
    #[cfg(target_os = "android")]
    fn awake(&self) {
        *self.state.lock().unwrap() = (true, false);
        self.fired.notify_one();
        let s = self.state.lock().unwrap();
        let (mut s, _) = self.cond.wait_timeout_while(s, MAX_AWAKE, |s| !s.1).unwrap();
        *s = (false, false);
    }
}
