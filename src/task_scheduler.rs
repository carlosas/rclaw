use crate::container::{run_container_agent, ContainerInput, RegisteredGroup};
use crate::db::{Db, Task};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use cron::Schedule;
use std::str::FromStr;
use std::sync::Arc;
use tokio::time::{self, Duration as TokioDuration};
use tracing::{error, info};

pub enum TaskSchedule {
    Cron(Schedule),
    Every(chrono::Duration),
}

pub struct TaskScheduler {
    db: Arc<Db>,
}

impl TaskScheduler {
    pub fn new(db: Arc<Db>) -> Self {
        TaskScheduler { db }
    }

    pub async fn run(&self) {
        info!("Task scheduler started.");
        let mut interval = time::interval(TokioDuration::from_secs(60)); // Check every minute

        loop {
            interval.tick().await;
            info!("Scheduler tick: Checking for tasks to run...");
            if let Err(e) = self.check_and_run_tasks().await {
                error!("Error in scheduler tick: {:?}", e);
            }
        }
    }

    async fn check_and_run_tasks(&self) -> Result<()> {
        let active_tasks = self.db.get_active_tasks()?;

        for mut task in active_tasks {
            let parsed_schedule = if task.schedule.starts_with("every ") {
                let parts: Vec<&str> = task.schedule.split_whitespace().collect();
                if parts.len() == 2 {
                    let amount_str = parts[1].trim_end_matches(|c: char| !c.is_ascii_digit());
                    let unit_str = parts[1].trim_start_matches(|c: char| c.is_ascii_digit());

                    if let Ok(amount) = amount_str.parse::<i64>() {
                        let duration = match unit_str {
                            "s" => {
                                Some(Duration::try_seconds(amount).unwrap_or_else(Duration::zero))
                            }
                            "m" => {
                                Some(Duration::try_minutes(amount).unwrap_or_else(Duration::zero))
                            }
                            "h" => Some(Duration::try_hours(amount).unwrap_or_else(Duration::zero)),
                            "d" => Some(Duration::try_days(amount).unwrap_or_else(Duration::zero)),
                            _ => None,
                        };

                        if let Some(d) = duration {
                            TaskSchedule::Every(d)
                        } else {
                            error!("Invalid 'every X' unit for task {}: {}", task.id, unit_str);
                            continue;
                        }
                    } else {
                        error!(
                            "Invalid 'every X' amount for task {}: {}",
                            task.id, amount_str
                        );
                        continue;
                    }
                } else {
                    error!(
                        "Invalid 'every X' format for task {}: {}",
                        task.id, task.schedule
                    );
                    continue;
                }
            } else {
                match Schedule::from_str(&task.schedule) {
                    Ok(s) => TaskSchedule::Cron(s),
                    Err(e) => {
                        error!("Invalid cron schedule for task {}: {}", task.id, e);
                        continue;
                    }
                }
            };

            let now_utc = Utc::now();
            let next_occurrence = match &parsed_schedule {
                TaskSchedule::Cron(schedule) => schedule.after(&now_utc).next(),
                TaskSchedule::Every(duration) => {
                    let last_run_dt = task.last_run.as_ref().and_then(|s| {
                        DateTime::parse_from_rfc3339(s)
                            .ok()
                            .map(|dt| dt.with_timezone(&Utc))
                    });

                    if let Some(last_run) = last_run_dt {
                        let mut calculated_next = last_run + *duration;
                        while calculated_next <= now_utc {
                            calculated_next = calculated_next + *duration;
                        }
                        Some(calculated_next)
                    } else {
                        // If no last_run, schedule for now + duration
                        Some(now_utc + *duration)
                    }
                }
            };

            if let Some(next_occurrence) = next_occurrence {
                // If next_run in DB is empty or older than calculated next_occurrence, update it
                let db_next_run = task.next_run.as_ref().and_then(|s| {
                    DateTime::parse_from_rfc3339(s)
                        .ok()
                        .map(|dt| dt.with_timezone(&Utc))
                });

                if db_next_run.is_none()
                    || (db_next_run.is_some() && db_next_run.unwrap() < next_occurrence)
                {
                    task.next_run = Some(next_occurrence.to_rfc3339());
                    // Persistir el cambio en next_run. Solo el campo next_run es mutable aquí, por lo que crearemos un nuevo Task
                    let task_to_update = Task {
                        id: task.id.clone(),
                        group_folder: task.group_folder.clone(),
                        prompt: task.prompt.clone(),
                        schedule: task.schedule.clone(),
                        last_run: task.last_run.clone(),
                        next_run: task.next_run.clone(),
                        status: task.status.clone(),
                    };
                    self.db.add_task(&task_to_update)?; // Usar add_task para actualizar
                    info!("Updated next_run for task {}: {:?}", task.id, task.next_run);
                }

                // Check if it's time to run the task
                if next_occurrence <= now_utc {
                    info!("Running task: {}", task.id);

                    let group_config = RegisteredGroup {
                        name: task.group_folder.clone(),
                        folder: task.group_folder.clone(),
                    };

                    let input = ContainerInput {
                        prompt: task.prompt.clone(),
                        session_id: "scheduled-task".to_string(),
                        group_folder: task.group_folder.clone(),
                        chat_jid: format!("scheduled-task-{}", task.id),
                        is_main: false,
                        is_scheduled_task: Some(true),
                    };

                    match tokio::task::spawn_blocking(move || {
                        run_container_agent(&group_config, &input)
                    })
                    .await
                    {
                        Ok(Ok(output)) => {
                            info!("Task {} agent finished: {:?}", task.id, output);
                            if let Some(res) = output.result {
                                info!("Task {} result: {}", task.id, res);
                            }
                            if let Some(err) = output.error {
                                error!("Task {} error: {}", task.id, err);
                            }
                        }
                        Ok(Err(e)) => {
                            error!("Task {} agent failed: {:?}", task.id, e);
                        }
                        Err(e) => {
                            error!("Task {} join error: {:?}", task.id, e);
                        }
                    }

                    // After running, update last_run and recalculate next_run
                    task.last_run = Some(now_utc.to_rfc3339());
                    let new_next_run_calc = match &parsed_schedule {
                        TaskSchedule::Cron(schedule) => {
                            schedule.after(&now_utc).next().map(|dt| dt.to_rfc3339())
                        }
                        TaskSchedule::Every(duration) => Some((now_utc + *duration).to_rfc3339()),
                    };
                    task.next_run = new_next_run_calc;

                    // Persistir el cambio en last_run y next_run
                    let task_to_update = Task {
                        id: task.id.clone(),
                        group_folder: task.group_folder.clone(),
                        prompt: task.prompt.clone(),
                        schedule: task.schedule.clone(),
                        last_run: task.last_run.clone(),
                        next_run: task.next_run.clone(),
                        status: task.status.clone(),
                    };
                    self.db.add_task(&task_to_update)?;
                    info!("Task {} completed, next run: {:?}", task.id, task.next_run);
                }
            } else {
                info!(
                    "No upcoming runs for task {}. Consider deactivating.",
                    task.id
                );
            }
        }
        Ok(())
    }
}
