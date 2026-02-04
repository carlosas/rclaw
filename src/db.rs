use rusqlite::{params, Connection, Result, OptionalExtension};
use std::path::Path;
use tracing::info;
use serde::{Serialize, Deserialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub group_folder: String,
    pub prompt: String,
    pub schedule: String, // Cron expression or "every X"
    pub last_run: Option<String>,
    pub next_run: Option<String>,
    pub status: String, // "active", "paused"
}

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Db { conn: Mutex::new(conn) };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        // Table for Auth (Key-Value store)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS auth_store (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
            [],
        )?;

        // Table for outgoing message queue (to handle async sending safely)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS message_queue (
                id INTEGER PRIMARY KEY,
                jid TEXT NOT NULL,
                content TEXT NOT NULL,
                status TEXT DEFAULT 'pending', -- pending, sent, failed
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                attempts INTEGER DEFAULT 0
            )",
            [],
        )?;

        // Table for scheduled tasks
        conn.execute(
            "CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                group_folder TEXT NOT NULL,
                prompt TEXT NOT NULL,
                schedule TEXT NOT NULL,
                last_run DATETIME,
                next_run DATETIME,
                status TEXT DEFAULT 'active'
            )",
            [],
        )?;

        info!("Database tables initialized.");
        Ok(())
    }

    // --- Auth Store Methods ---
    pub fn get_auth_key(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT value FROM auth_store WHERE key = ?1",
            params![key],
            |row| row.get(0),
        ).optional()
    }

    pub fn set_auth_key(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO auth_store (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn delete_auth_key(&self, key: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM auth_store WHERE key = ?1",
            params![key],
        )?;
        Ok(())
    }

    // --- Message Queue Methods ---
    pub fn queue_message(&self, jid: &str, content: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO message_queue (jid, content) VALUES (?1, ?2)",
            params![jid, content],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_pending_messages(&self) -> Result<Vec<(i64, String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, jid, content FROM message_queue WHERE status = 'pending' ORDER BY created_at ASC LIMIT 10"
        )?;
        
        let msgs = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<Result<Vec<_>>>()?;
        
        Ok(msgs)
    }

    pub fn mark_message_sent(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE message_queue SET status = 'sent' WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    // --- Task Methods ---
    pub fn add_task(&self, task: &Task) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO tasks (id, group_folder, prompt, schedule, last_run, next_run, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                task.id, 
                task.group_folder, 
                task.prompt, 
                task.schedule, 
                task.last_run, 
                task.next_run, 
                task.status
            ],
        )?;
        Ok(())
    }
    
    pub fn get_active_tasks(&self) -> Result<Vec<Task>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, group_folder, prompt, schedule, last_run, next_run, status 
             FROM tasks WHERE status = 'active'"
        )?;

        let tasks = stmt.query_map([], |row| {
            Ok(Task {
                id: row.get(0)?,
                group_folder: row.get(1)?,
                prompt: row.get(2)?,
                schedule: row.get(3)?,
                last_run: row.get(4)?,
                next_run: row.get(5)?,
                status: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>>>()?;

        Ok(tasks)
    }
}