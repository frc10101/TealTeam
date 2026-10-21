//! Account administration (G1): what `/admin/users` reads and changes.
//!
//! Before this, the first sign-up was the only admin and every later lead
//! scout or coach took hand-written SQL on the Pi. Two rules live here rather
//! than in the handler, so no caller can skip them:
//!
//! * the last admin cannot stop being one -- a deployment with no admin has
//!   nobody left who can grant anybody anything, and getting one back is
//!   exactly the hand-written SQL this item exists to end;
//! * a password reset ends every session and offline token the account
//!   holds, in the same transaction as the new hash, so whoever knew the old
//!   password is out the moment it stops working.

use chrono::{DateTime, Utc};
use sqlx::Row;
use tt_core::user::Roles;
use tt_repo::Result;

use crate::SqliteRepo;
use crate::token_key::end_everywhere;
use crate::users::{from_sql, query_err, to_sql};

/// One row of the account list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub team_number: Option<i32>,
    pub roles: Roles,
    pub last_login_at: Option<DateTime<Utc>>,
}

/// What [`SqliteRepo::set_account`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountChange {
    Saved,
    /// Refused: it would have left nobody an admin. Nothing was changed.
    LastAdmin,
    NoSuchAccount,
}

impl SqliteRepo {
    /// Every account, by name.
    pub async fn accounts(&self) -> Result<Vec<Account>> {
        let rows = sqlx::query(
            "SELECT id, name, email, team_number, is_admin, is_lead_scout, is_coach, \
                    last_login_at \
             FROM users ORDER BY name COLLATE NOCASE, id",
        )
        .fetch_all(self.pool())
        .await
        .map_err(|e| query_err("listing accounts", e))?;
        Ok(rows
            .iter()
            .map(|row| Account {
                id: row.get("id"),
                name: row.get("name"),
                email: row.get("email"),
                team_number: row.get("team_number"),
                roles: Roles {
                    is_admin: row.get::<i64, _>("is_admin") != 0,
                    is_lead_scout: row.get::<i64, _>("is_lead_scout") != 0,
                    is_coach: row.get::<i64, _>("is_coach") != 0,
                },
                last_login_at: row
                    .get::<Option<String>, _>("last_login_at")
                    .as_deref()
                    .and_then(from_sql),
            })
            .collect())
    }

    /// Set an account's roles and team number together.
    ///
    /// The last-admin check is inside the one `UPDATE`, not a count read
    /// first, so two admins demoting each other at once cannot both succeed.
    pub async fn set_account(
        &self,
        id: i64,
        team_number: Option<i32>,
        roles: Roles,
        now: DateTime<Utc>,
    ) -> Result<AccountChange> {
        let updated = sqlx::query(
            "UPDATE users SET team_number = ?, is_admin = ?, is_lead_scout = ?, is_coach = ?, \
                    updated_at = ? \
             WHERE id = ? \
               AND (? = 1 OR is_admin = 0 \
                    OR (SELECT COUNT(*) FROM users WHERE is_admin = 1) > 1)",
        )
        .bind(team_number)
        .bind(roles.is_admin as i64)
        .bind(roles.is_lead_scout as i64)
        .bind(roles.is_coach as i64)
        .bind(to_sql(now))
        .bind(id)
        .bind(roles.is_admin as i64)
        .execute(self.pool())
        .await
        .map_err(|e| query_err("changing an account", e))?;
        if updated.rows_affected() == 1 {
            return Ok(AccountChange::Saved);
        }
        let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(|e| query_err("looking up an account", e))?;
        Ok(match exists {
            Some(_) => AccountChange::LastAdmin,
            None => AccountChange::NoSuchAccount,
        })
    }

    /// Give `id` a new password hash and end every session and offline token
    /// it holds (as [`SqliteRepo::sign_out_everywhere`] does), in one
    /// transaction. `false` if there is no such account.
    pub async fn reset_password(
        &self,
        id: i64,
        hash: &str,
        by: i64,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|e| query_err("resetting a password", e))?;
        let updated =
            sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
                .bind(hash)
                .bind(to_sql(now))
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| query_err("resetting a password", e))?;
        if updated.rows_affected() == 0 {
            return Ok(false);
        }
        end_everywhere(&mut tx, id, by, now).await?;
        tx.commit()
            .await
            .map_err(|e| query_err("resetting a password", e))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    const ADMIN: Roles = Roles {
        is_admin: true,
        is_lead_scout: false,
        is_coach: false,
    };

    async fn repo_with(people: &[(i64, &str, bool)]) -> SqliteRepo {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        crate::migrate::apply(repo.pool()).await.expect("migrate");
        let now = to_sql(Utc::now());
        for (id, name, admin) in people {
            sqlx::query(
                "INSERT INTO users (id, email, name, password_hash, is_admin, created_at, updated_at) \
                 VALUES (?, ?, ?, 'old-hash', ?, ?, ?)",
            )
            .bind(id)
            .bind(format!("{name}@example.com"))
            .bind(name)
            .bind(*admin as i64)
            .bind(&now)
            .bind(&now)
            .execute(repo.pool())
            .await
            .unwrap();
        }
        repo
    }

    #[tokio::test]
    async fn the_list_carries_team_roles_and_last_sign_in() {
        let repo = repo_with(&[(1, "Sam", true), (2, "Kim", false)]).await;
        let at = Utc::now();
        repo.record_login_impl(2, at).await.unwrap();
        repo.set_account(
            2,
            Some(10101),
            Roles {
                is_coach: true,
                ..Roles::SCOUT
            },
            at,
        )
        .await
        .unwrap();

        let listed = repo.accounts().await.unwrap();
        let names: Vec<&str> = listed.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Kim", "Sam"], "by name");
        assert_eq!(listed[0].team_number, Some(10101));
        assert!(listed[0].roles.is_coach && !listed[0].roles.is_admin);
        assert_eq!(listed[0].last_login_at, from_sql(&to_sql(at)));
        assert_eq!(listed[1].last_login_at, None, "never signed in");
    }

    #[tokio::test]
    async fn the_last_admin_cannot_stop_being_one() {
        let repo = repo_with(&[(1, "Sam", true), (2, "Kim", false)]).await;
        let now = Utc::now();
        assert_eq!(
            repo.set_account(1, Some(5), Roles::SCOUT, now)
                .await
                .unwrap(),
            AccountChange::LastAdmin
        );
        let sam = repo.user_by_id_impl(1).await.unwrap().unwrap();
        assert!(sam.roles.is_admin, "nothing changed");
        assert_eq!(sam.team_number, None, "not even the team number");

        // Keeping admin while changing the rest is fine.
        assert_eq!(
            repo.set_account(1, Some(5), ADMIN, now).await.unwrap(),
            AccountChange::Saved
        );

        // With a second admin, either one may step down -- but not both.
        repo.set_account(2, None, ADMIN, now).await.unwrap();
        assert_eq!(
            repo.set_account(1, None, Roles::SCOUT, now).await.unwrap(),
            AccountChange::Saved
        );
        assert_eq!(
            repo.set_account(2, None, Roles::SCOUT, now).await.unwrap(),
            AccountChange::LastAdmin
        );
        assert_eq!(
            repo.set_account(99, None, Roles::SCOUT, now).await.unwrap(),
            AccountChange::NoSuchAccount
        );
    }

    #[tokio::test]
    async fn a_reset_password_ends_every_session_and_token_of_that_account_only() {
        let repo = repo_with(&[(1, "Sam", true), (2, "Kim", false)]).await;
        let now = Utc::now();
        for id in [1, 2] {
            sqlx::query(
                "INSERT INTO sessions (id, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)",
            )
            .bind(format!("session-{id}"))
            .bind(id)
            .bind(to_sql(now))
            .bind(to_sql(now + TimeDelta::hours(1)))
            .execute(repo.pool())
            .await
            .unwrap();
        }

        assert!(repo.reset_password(2, "new-hash", 1, now).await.unwrap());
        assert_eq!(
            repo.password_hash_impl(2).await.unwrap().as_deref(),
            Some("new-hash")
        );
        assert_eq!(repo.token_epoch(2).await.unwrap(), 1);
        let left: Vec<i64> = sqlx::query_scalar("SELECT user_id FROM sessions")
            .fetch_all(repo.pool())
            .await
            .unwrap();
        assert_eq!(left, [1], "only Kim's sessions end");
        assert_eq!(repo.token_epoch(1).await.unwrap(), 0);

        assert!(!repo.reset_password(99, "x", 1, now).await.unwrap());
    }
}
