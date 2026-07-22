use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rusqlite::Connection;

use crate::StorageError;

/// Thread-safe handle to a SQLite connection. Clone is cheap (Arc clone).
///
/// # Non-reentrancy
///
/// Do NOT call `with_conn` or `with_conn_sync` from inside a closure passed
/// to these methods. Pass the `&Connection` reference to inner helpers instead.
/// Same-thread reentry is detected and panics; cross-thread reentry can deadlock.
#[derive(Clone)]
pub struct DbHandle {
    conn: Arc<Mutex<Connection>>,
}

impl std::fmt::Debug for DbHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbHandle").finish_non_exhaustive()
    }
}

thread_local! {
    static ACTIVE_HANDLES: std::cell::RefCell<std::collections::HashSet<usize>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

struct ReentrancyGuard {
    key: usize,
}

impl Drop for ReentrancyGuard {
    fn drop(&mut self) {
        ACTIVE_HANDLES.with(|set| {
            set.borrow_mut().remove(&self.key);
        });
    }
}

fn enter_guard(key: usize) -> ReentrancyGuard {
    ACTIVE_HANDLES.with(|set| {
        if !set.borrow_mut().insert(key) {
            panic!("DbHandle re-entered — pass &Connection instead");
        }
    });
    ReentrancyGuard { key }
}

fn lock_connection(conn: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    conn.lock().unwrap_or_else(|poisoned| {
        let conn = PoisonError::into_inner(poisoned);

        // D1:124 requires poison recovery. Rusqlite transaction guards roll back on unwind in
        // the normal case; this defensive ROLLBACK covers a hypothetical raw-BEGIN panicker so a
        // recovered connection can never silently ride an open transaction.
        if !conn.is_autocommit() {
            conn.execute_batch("ROLLBACK").unwrap_or_else(|error| {
                // A connection SQLite cannot roll back is genuinely unusable, so a loud panic is
                // more honest than lending it back to callers.
                panic!(
                    "DbHandle poison recovery could not roll back an open SQLite transaction; \
                     connection is unusable: {error}"
                )
            });
        }

        conn
    })
}

impl DbHandle {
    pub(crate) fn new(conn: Connection) -> Self {
        Self {
            conn: Arc::new(Mutex::new(conn)),
        }
    }

    fn identity(&self) -> usize {
        Arc::as_ptr(&self.conn) as usize
    }

    pub async fn with_conn<F, T>(&self, f: F) -> Result<T, StorageError>
    where
        F: FnOnce(&Connection) -> Result<T, StorageError> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        let key = self.identity();
        tokio::task::spawn_blocking(move || {
            let _guard = enter_guard(key);
            let lock = lock_connection(&conn);
            f(&lock)
        })
        .await
        .expect("DbHandle spawn_blocking task panicked")
    }

    pub fn with_conn_sync<F, T>(&self, f: F) -> Result<T, StorageError>
    where
        F: FnOnce(&Connection) -> Result<T, StorageError>,
    {
        let _guard = enter_guard(self.identity());
        let lock = lock_connection(&self.conn);
        f(&lock)
    }
}

#[cfg(test)]
#[path = "../tests/unit/handle_tests.rs"]
mod tests;
