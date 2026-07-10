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
mod tests {
    use super::*;

    #[test]
    fn with_conn_sync_executes_query() {
        let conn = Connection::open_in_memory().unwrap();
        let handle = DbHandle::new(conn);
        let result = handle
            .with_conn_sync(|c| {
                let n: i64 = c.query_row("SELECT 42", [], |row| row.get(0))?;
                Ok(n)
            })
            .unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn with_conn_sync_poison_recovery_rolls_back_open_transaction() {
        let conn = Connection::open_in_memory().unwrap();
        let handle = DbHandle::new(conn);
        handle
            .with_conn_sync(|c| {
                c.execute_batch("CREATE TABLE readings (value INTEGER NOT NULL)")?;
                Ok(())
            })
            .unwrap();

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<(), StorageError> = handle.with_conn_sync(|c| {
                c.execute_batch("BEGIN; INSERT INTO readings (value) VALUES (1)")?;
                panic!("intentional test panic");
            });
        }));
        assert!(panic.is_err());

        handle
            .with_conn_sync(|c| {
                assert!(c.is_autocommit());
                let count: i64 =
                    c.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))?;
                assert_eq!(count, 0);

                let tx = c.unchecked_transaction()?;
                tx.execute("INSERT INTO readings (value) VALUES (2)", [])?;
                tx.commit()?;
                Ok(())
            })
            .unwrap();

        let count: i64 = handle
            .with_conn_sync(|c| {
                c.query_row("SELECT COUNT(*) FROM readings", [], |row| row.get(0))
                    .map_err(StorageError::from)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn with_conn_executes_query() {
        let conn = Connection::open_in_memory().unwrap();
        let handle = DbHandle::new(conn);
        let result = handle
            .with_conn(|c| {
                let n: i64 = c.query_row("SELECT 1 + 1", [], |row| row.get(0))?;
                Ok(n)
            })
            .await
            .unwrap();
        assert_eq!(result, 2);
    }

    #[tokio::test]
    async fn with_conn_recovers_after_closure_panics() {
        let conn = Connection::open_in_memory().unwrap();
        let handle = DbHandle::new(conn);
        let panicking_handle = handle.clone();

        let panic = tokio::spawn(async move {
            let _: Result<(), StorageError> = panicking_handle
                .with_conn(|_c| panic!("intentional test panic"))
                .await;
        })
        .await
        .unwrap_err();
        assert!(panic.is_panic());

        let result = handle
            .with_conn(|c| {
                let n: i64 = c.query_row("SELECT 42", [], |row| row.get(0))?;
                Ok(n)
            })
            .await
            .unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    #[should_panic(expected = "DbHandle re-entered")]
    fn with_conn_sync_reentry_panics() {
        let conn = Connection::open_in_memory().unwrap();
        let handle = DbHandle::new(conn);
        let handle_clone = handle.clone();
        handle
            .with_conn_sync(|_c| {
                handle_clone.with_conn_sync(|_c2| Ok(())).unwrap();
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn cross_thread_contention_both_succeed() {
        let conn = Connection::open_in_memory().unwrap();
        let handle = DbHandle::new(conn);
        let handle2 = handle.clone();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let barrier2 = barrier.clone();

        let t1 = std::thread::spawn(move || {
            handle.with_conn_sync(|c| {
                barrier.wait();
                let n: i64 = c.query_row("SELECT 1", [], |row| row.get(0))?;
                std::thread::sleep(std::time::Duration::from_millis(50));
                Ok(n)
            })
        });

        let t2 = std::thread::spawn(move || {
            barrier2.wait();
            std::thread::sleep(std::time::Duration::from_millis(10));
            handle2.with_conn_sync(|c| {
                let n: i64 = c.query_row("SELECT 2", [], |row| row.get(0))?;
                Ok(n)
            })
        });

        assert_eq!(t1.join().unwrap().unwrap(), 1);
        assert_eq!(t2.join().unwrap().unwrap(), 2);
    }

    #[tokio::test]
    async fn concurrent_async_contention_succeeds() {
        let conn = Connection::open_in_memory().unwrap();
        let handle = DbHandle::new(conn);
        let h1 = handle.clone();
        let h2 = handle.clone();

        let (r1, r2) = tokio::join!(
            h1.with_conn(|c| {
                let n: i64 = c.query_row("SELECT 10", [], |row| row.get(0))?;
                Ok(n)
            }),
            h2.with_conn(|c| {
                let n: i64 = c.query_row("SELECT 20", [], |row| row.get(0))?;
                Ok(n)
            }),
        );

        assert_eq!(r1.unwrap(), 10);
        assert_eq!(r2.unwrap(), 20);
    }
}
