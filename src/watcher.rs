use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::git;

const DEBOUNCE: Duration = Duration::from_millis(500);
const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const MAX_EVENT_LATENCY: Duration = Duration::from_secs(2);
const RECONCILE_INTERVAL: Duration = Duration::from_secs(30);

pub struct AutoRefresh {
    repository: PathBuf,
    watched_worktree: Option<PathBuf>,
    backend: Option<WatchBackend>,
    schedule: RefreshSchedule,
}

impl AutoRefresh {
    pub fn new(repository: &Path, worktree: Option<&Path>) -> Self {
        let now = Instant::now();
        let mut refresh = Self {
            repository: repository.to_path_buf(),
            watched_worktree: None,
            backend: None,
            schedule: RefreshSchedule::new(now),
        };
        refresh.watch_worktree(worktree, now);
        refresh
    }

    pub fn watch_worktree(&mut self, worktree: Option<&Path>, now: Instant) {
        if self.backend.is_some() && self.watched_worktree.as_deref() == worktree {
            return;
        }
        self.backend = WatchBackend::new(&self.repository, worktree).ok();
        self.watched_worktree = worktree.map(Path::to_path_buf);
        self.schedule.mark_refreshed(now);
    }

    pub fn refresh_paths(&mut self, now: Instant) -> Option<Vec<PathBuf>> {
        if let Some(backend) = &mut self.backend {
            while let Some(event) = backend.next_event() {
                if let Some(event) = event.filter(is_relevant_event) {
                    self.schedule.note_event(now, event.paths);
                }
            }
        }
        self.schedule
            .should_refresh(now)
            .then(|| self.schedule.pending_paths.clone())
    }

    pub fn mark_refreshed(&mut self, now: Instant) {
        self.schedule.mark_refreshed(now);
    }
}

struct WatchBackend {
    _watcher: RecommendedWatcher,
    receiver: Receiver<notify::Result<Event>>,
}

impl WatchBackend {
    fn new(repository: &Path, worktree: Option<&Path>) -> notify::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(sender)?;
        let common_dir = git::common_git_dir(repository)
            .map_err(|error| notify::Error::generic(&error.to_string()))?;
        watcher.watch(&common_dir, RecursiveMode::Recursive)?;

        if let Some(worktree) = worktree
            && worktree.is_dir()
            && worktree != common_dir
        {
            watcher.watch(worktree, RecursiveMode::Recursive)?;
        }

        Ok(Self {
            _watcher: watcher,
            receiver,
        })
    }

    fn next_event(&mut self) -> Option<Option<Event>> {
        match self.receiver.try_recv() {
            Ok(Ok(event)) => Some(Some(event)),
            Ok(Err(_)) => Some(None),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

fn is_relevant_event(event: &Event) -> bool {
    !matches!(event.kind, EventKind::Access(_))
}

#[derive(Clone, Debug)]
struct RefreshSchedule {
    pending_since: Option<Instant>,
    last_event: Option<Instant>,
    pending_paths: Vec<PathBuf>,
    last_refresh: Instant,
    next_reconcile: Instant,
}

impl RefreshSchedule {
    fn new(now: Instant) -> Self {
        Self {
            pending_since: None,
            last_event: None,
            pending_paths: Vec::new(),
            last_refresh: now,
            next_reconcile: now + RECONCILE_INTERVAL,
        }
    }

    fn note_event(&mut self, now: Instant, paths: Vec<PathBuf>) {
        self.pending_since.get_or_insert(now);
        self.last_event = Some(now);
        self.pending_paths.extend(paths);
    }

    fn should_refresh(&self, now: Instant) -> bool {
        if now >= self.next_reconcile {
            return true;
        }
        let (Some(pending_since), Some(last_event)) = (self.pending_since, self.last_event) else {
            return false;
        };
        let rate_limit_elapsed = now.duration_since(self.last_refresh) >= MIN_REFRESH_INTERVAL;
        let burst_settled = now.duration_since(last_event) >= DEBOUNCE;
        let max_latency_elapsed = now.duration_since(pending_since) >= MAX_EVENT_LATENCY;
        rate_limit_elapsed && (burst_settled || max_latency_elapsed)
    }

    fn mark_refreshed(&mut self, now: Instant) {
        self.pending_since = None;
        self.last_event = None;
        self.pending_paths.clear();
        self.last_refresh = now;
        self.next_reconcile = now + RECONCILE_INTERVAL;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debounces_events_and_rate_limits_refreshes() {
        let start = Instant::now();
        let mut schedule = RefreshSchedule::new(start);
        schedule.note_event(start + Duration::from_millis(100), Vec::new());
        assert!(!schedule.should_refresh(start + Duration::from_millis(700)));
        assert!(schedule.should_refresh(start + Duration::from_secs(1)));

        schedule.mark_refreshed(start + Duration::from_secs(1));
        schedule.note_event(start + Duration::from_millis(1_100), Vec::new());
        assert!(!schedule.should_refresh(start + Duration::from_millis(1_800)));
        assert!(schedule.should_refresh(start + Duration::from_secs(2)));
    }

    #[test]
    fn refreshes_during_a_continuous_event_stream() {
        let start = Instant::now();
        let mut schedule = RefreshSchedule::new(start);
        for millis in [100, 400, 800, 1_200, 1_600, 2_000] {
            schedule.note_event(start + Duration::from_millis(millis), Vec::new());
        }
        assert!(schedule.should_refresh(start + Duration::from_millis(2_100)));
    }

    #[test]
    fn retains_paths_for_the_refresh_that_follows_a_file_event() {
        let start = Instant::now();
        let mut schedule = RefreshSchedule::new(start);
        let path = PathBuf::from("src/main.rs");
        schedule.note_event(start + Duration::from_millis(100), vec![path.clone()]);

        assert!(schedule.should_refresh(start + Duration::from_secs(1)));
        assert_eq!(schedule.pending_paths, vec![path]);

        schedule.mark_refreshed(start + Duration::from_secs(1));
        assert!(schedule.pending_paths.is_empty());
    }

    #[test]
    fn reconciles_periodically_without_events() {
        let start = Instant::now();
        let schedule = RefreshSchedule::new(start);
        assert!(!schedule.should_refresh(start + Duration::from_secs(29)));
        assert!(schedule.should_refresh(start + RECONCILE_INTERVAL));
    }
}
