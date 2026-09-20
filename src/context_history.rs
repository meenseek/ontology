//! PostgreSQL recovery gate for current context reads and imported-file writes.
//! This is storage bookkeeping; Core alone owns candidate validation and acceptance.
use crate::{domain::Error, store::Store};
use sqlx::{Postgres, Transaction};

impl Store {
    pub(crate) async fn lock_context(
        &self,
        exclusive: bool,
    ) -> Result<Transaction<'static, Postgres>, Error> {
        self.count(1);
        let mut tx = self.pool().begin().await.map_err(|_| Error::Storage)?;
        let result = self.context_gate_in(&mut tx, exclusive).await;
        if let Err(error) = result {
            return self.finish_context(tx, Err(error)).await;
        }
        Ok(tx)
    }

    pub(crate) async fn context_gate_in(
        &self,
        conn: &mut sqlx::PgConnection,
        exclusive: bool,
    ) -> Result<(), Error> {
        self.count(1);
        let acquired: bool = sqlx::query_scalar(if exclusive {
            "SELECT pg_try_advisory_xact_lock(478310003)"
        } else {
            "SELECT pg_try_advisory_xact_lock_shared(478310003)"
        })
        .fetch_one(&mut *conn)
        .await
        .map_err(|_| Error::Storage)?;
        if !acquired {
            return Err(Error::ContextPending);
        }
        self.count(1);
        let pending: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM context_apply_batches WHERE state IN ('pending','committed'))",
            )
            .fetch_one(&mut *conn)
            .await
            .map_err(|_| Error::Storage)?;
        if pending {
            return Err(Error::ContextPending);
        }
        Ok(())
    }

    pub(crate) async fn finish_context<T>(
        &self,
        tx: Transaction<'_, Postgres>,
        result: Result<T, Error>,
    ) -> Result<T, Error> {
        self.count(1);
        if result.is_ok() {
            tx.commit().await.map_err(|_| Error::Storage)?;
        } else {
            tx.rollback().await.map_err(|_| Error::Storage)?;
        }
        result
    }
}
