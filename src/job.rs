use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Datelike, Duration, Local, Months, Utc, Weekday};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{task::JoinHandle, time::sleep_until};

use crate::notifier::{Notification, Notifier};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatTemplate {
    Daily,
    Weekdays,
    Weekends,
    Weekly,
    Biweekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatFrequency {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Repeat {
    Template(RepeatTemplate),
    Custom { frequency: RepeatFrequency, every: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub run_at: DateTime<Utc>,
    #[serde(default)]
    pub repeat: Option<Repeat>,
    #[serde(default)]
    pub end_repeat: Option<DateTime<Utc>>,
    pub method: String,
    pub url: String,
    pub body: Option<Value>,
}

#[derive(Clone)]
pub struct JobManager {
    pub notifier: Notifier,
    pub jobs: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
    pub file_path: PathBuf,
    pub current_jobs: Arc<Mutex<Vec<Job>>>,
    pub max_late_secs: u64,
}

fn seconds_until(run_at: DateTime<Utc>) -> i64 {
    let now = Utc::now();
    let delta = run_at.signed_duration_since(now);
    delta.num_seconds()
}

fn next_run_time(job: &Job, last_run: DateTime<Utc>, always_advance: bool) -> Option<DateTime<Utc>> {
    let now = Utc::now();

    fn days_until_next(last_run: DateTime<Utc>, now: DateTime<Utc>, every: i64, always_advance: bool) -> i64 {
        let mut periods = (now.signed_duration_since(last_run).num_days() / every).max(0);
        if always_advance || last_run <= now {
            periods += 1;
        }
        periods
    }

    fn weeks_until_next(last_run: DateTime<Utc>, now: DateTime<Utc>, every: i64, always_advance: bool) -> i64 {
        let mut periods = (now.signed_duration_since(last_run).num_weeks() / every).max(0);
        if always_advance || last_run <= now {
            periods += 1;
        }
        periods
    }

    fn months_until_next(last_run: DateTime<Utc>, now: DateTime<Utc>, every: i32, always_advance: bool) -> i32 {
        let mut months = (now.year() - last_run.year()) * 12 + (now.month() as i32 - last_run.month() as i32);
        months /= every;
        if always_advance || last_run.checked_add_months(Months::new((months * every) as u32)) <= Some(now) {
            months += 1;
        }
        months
    }

    fn next_weekday_after(
        mut dt: DateTime<Utc>,
        now: DateTime<Utc>,
        condition: impl Fn(Weekday) -> bool,
    ) -> DateTime<Utc> {
        loop {
            dt += Duration::days(1);
            if condition(dt.weekday()) && dt > now {
                break dt;
            }
        }
    }

    match &job.repeat {
        Some(Repeat::Template(template)) => match template {
            RepeatTemplate::Daily => Some(last_run + Duration::days(days_until_next(last_run, now, 1, always_advance))),
            RepeatTemplate::Weekdays => Some(next_weekday_after(last_run, now, |w| {
                w != Weekday::Sat && w != Weekday::Sun
            })),
            RepeatTemplate::Weekends => Some(next_weekday_after(last_run, now, |w| {
                w == Weekday::Sat || w == Weekday::Sun
            })),
            RepeatTemplate::Weekly => {
                Some(last_run + Duration::weeks(weeks_until_next(last_run, now, 1, always_advance)))
            }
            RepeatTemplate::Biweekly => {
                Some(last_run + Duration::weeks(weeks_until_next(last_run, now, 2, always_advance) * 2))
            }
            RepeatTemplate::Monthly => {
                let months = months_until_next(last_run, now, 1, always_advance);
                last_run.checked_add_months(Months::new(months as u32))
            }
            RepeatTemplate::Yearly => {
                let months = months_until_next(last_run, now, 12, always_advance);
                last_run.checked_add_months(Months::new(months as u32))
            }
        },
        Some(Repeat::Custom { frequency, every }) => match frequency {
            RepeatFrequency::Daily => Some(
                last_run
                    + Duration::days(days_until_next(last_run, now, *every as i64, always_advance) * *every as i64),
            ),
            RepeatFrequency::Weekly => Some(
                last_run
                    + Duration::weeks(weeks_until_next(last_run, now, *every as i64, always_advance) * *every as i64),
            ),
            RepeatFrequency::Monthly => {
                let months = months_until_next(last_run, now, *every as i32, always_advance) * *every as i32;
                last_run.checked_add_months(Months::new(months as u32))
            }
            RepeatFrequency::Yearly => {
                let months = months_until_next(last_run, now, *every as i32 * 12, always_advance) * *every as i32 * 12;
                last_run.checked_add_months(Months::new(months as u32))
            }
        },
        None => None,
    }
}

impl JobManager {
    pub fn new(notifier: Notifier, file_path: &Path, max_late_secs: u64) -> Self {
        Self {
            notifier,
            jobs: Arc::new(Mutex::new(HashMap::new())),
            file_path: file_path.to_path_buf(),
            current_jobs: Arc::new(Mutex::new(Vec::new())),
            max_late_secs,
        }
    }

    fn load_jobs(&self) -> Vec<Job> {
        let data = fs::read_to_string(&self.file_path).unwrap_or_default();
        match serde_json::from_str::<Vec<Job>>(&data) {
            Ok(jobs) => jobs,
            Err(error) => {
                tracing::warn!("Failed to load jobs: {:?}", error);
                Default::default()
            }
        }
    }

    pub fn schedule_jobs(&self) {
        let jobs = self.load_jobs();

        // Keep a copy for jobs API
        {
            let mut current = self.current_jobs.lock().unwrap();
            *current = jobs.clone();
        }

        {
            let mut handles = self.jobs.lock().unwrap();

            // Cancel all old jobs
            for (_, handle) in handles.drain() {
                handle.abort();
            }
        }

        for job in jobs {
            // Skip expired jobs
            if job.repeat.is_none() {
                let delay_secs = seconds_until(job.run_at);
                let expire_secs = -(self.max_late_secs as i64);

                if delay_secs < expire_secs {
                    tracing::info!(
                        "Skipping job [{}]: expired (scheduled {}, now {}, max_late_secs {})",
                        job.id,
                        job.run_at.with_timezone(&Local),
                        Utc::now().with_timezone(&Local),
                        self.max_late_secs
                    );
                    continue;
                }
            }

            self.spawn_job(job.clone());
        }

        // Notify
        self.notifier.notify(Notification::JobsUpdated);
    }

    fn spawn_job(&self, mut job: Job) {
        if job.repeat.is_some() {
            let now = Utc::now();

            // Determine the next valid run time if first run is in the past
            if job.run_at <= now
                && let Some(next) = next_run_time(&job, job.run_at, false)
            {
                // Set next run
                job.run_at = next;
            }

            // If there is an end_repeat and it's passed, stop scheduling
            if let Some(end_repeat) = job.end_repeat
                && (job.run_at > end_repeat || now >= end_repeat)
            {
                tracing::info!(
                    "Job [{}] expired and end_repeat at {} reached, skipping..",
                    job.id,
                    end_repeat.with_timezone(&Local)
                );
                return;
            }

            tracing::info!(
                "Job [{}] next repeated run will be at {}",
                job.id,
                job.run_at.with_timezone(&Local)
            );
        }

        let manager = self.clone();
        let notifier = self.notifier.clone();
        let id = job.id.clone();

        let handle = tokio::spawn(async move {
            // Schedule the job
            let delay_secs = seconds_until(job.run_at);
            let delay = if delay_secs > 0 { delay_secs as u64 } else { 0 };
            let when = std::time::Instant::now() + std::time::Duration::from_secs(delay);

            let id = job.id.clone();

            // Sleep
            tracing::trace!("Job [{}] idle for {} seconds", id, delay);
            sleep_until(when.into()).await;
            tracing::info!("Running job {}", id);

            // Notify
            notifier.notify(Notification::RunningJob { id: id.clone() });

            let client = reqwest::Client::new();

            let request = match job.method.to_uppercase().as_str() {
                "GET" => client.get(&job.url),
                "POST" => {
                    if let Some(ref b) = job.body {
                        client.post(&job.url).json(&b)
                    } else {
                        client.post(&job.url)
                    }
                }
                "PUT" => {
                    if let Some(ref b) = job.body {
                        client.put(&job.url).json(&b)
                    } else {
                        client.put(&job.url)
                    }
                }
                "DELETE" => client.delete(&job.url),
                _ => {
                    tracing::warn!("Job [{}] has unsupported method: {}", id, job.method);
                    return;
                }
            };

            match request.send().await {
                Ok(resp) => tracing::info!("Job [{}] executed -> {}", id, resp.status()),
                Err(error) => tracing::warn!("Job [{}] failed: {}", id, error),
            }

            // Schedule next run if repeating
            if let Some(next) = next_run_time(&job, job.run_at, true)
                && job.end_repeat.is_none_or(|end| next <= end)
            {
                let mut next_job = job.clone();
                next_job.run_at = next;

                tracing::info!(
                    "Job [{}] will be repeated again at {}",
                    job.id,
                    next.with_timezone(&Local)
                );

                manager.spawn_job(next_job);
            }
        });

        let mut handles = self.jobs.lock().unwrap();
        if let Some(prev) = handles.insert(id.clone(), handle) {
            // Stop previous run
            prev.abort();
        }
    }

    pub fn watch(&self) {
        let mgr = self.clone();
        let dir = self
            .file_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();
        let file_name = self.file_path.file_name().unwrap().to_os_string();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let (tx, mut rx) = tokio::sync::mpsc::channel(1);

                let mut watcher: RecommendedWatcher = RecommendedWatcher::new(
                    move |res| {
                        let _ = tx.blocking_send(res);
                    },
                    notify::Config::default(),
                )
                .unwrap();

                watcher.watch(&dir, RecursiveMode::NonRecursive).unwrap();

                while let Some(Ok(event)) = rx.recv().await {
                    // Any create/modify/remove event in the directory
                    if dir.join(&file_name).exists()
                        && matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_))
                    {
                        tracing::info!("Jobs file changed or created, reloading jobs...");
                        mgr.schedule_jobs();
                    }
                }
            });
        });
    }
}
