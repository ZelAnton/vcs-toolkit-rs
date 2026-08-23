//! Transaction and cancellation-safe operation-log rollback support.

use super::*;

/// The dedicated deadline the concurrency-safe rollback ([`Jj::rollback_to`])
/// bounds each of its own commands with. Set explicitly (not inherited) so a
/// cleanup that follows a *cancelled or timed-out* operation still runs on a full,
/// fresh budget rather than a spent one — a local `op log` / `op restore` is quick,
/// so this is a generous ceiling, not a tight bound.
const ROLLBACK_TIMEOUT: Duration = Duration::from_secs(30);

/// How many recent operations the divergence probe ([`Jj::rollback_to`]) walks back
/// through, as jj's `--limit`, looking for the captured pre-operation. Generous — a
/// single failed transaction records a handful of operations — so an honest rollback
/// is only ever refused for genuine divergence, not for depth. If the captured
/// operation is not within this window the probe treats the range as unverifiable
/// and refuses to revert (see [`Rollback::SkippedDiverged`]).
const ROLLBACK_PROBE_LIMIT: &str = "256";

/// What the concurrency-safe op-log rollback did after a mutation failed — the
/// outcome [`Jj::rollback_to`] returns and [`Jj::transaction`] reports on its
/// [`TransactionError`]. It lets a caller tell a completed rollback apart from one
/// that was deliberately **refused** (a concurrent process's work would have been
/// clobbered) or one that **failed**, instead of guessing by re-probing the op log.
#[derive(Debug)]
#[non_exhaustive]
pub enum Rollback {
    /// The repo is back at the captured operation — either `op restore` ran, or the
    /// closure failed before recording any operation, so nothing needed undoing.
    Restored,
    /// The rollback was **skipped** to avoid clobbering a concurrent process: the
    /// operation log diverged between the capture and the restore (jj reconciled a
    /// foreign operation with a "reconcile divergent operations" merge), so restoring
    /// to the captured operation would have silently reverted that work. The repo is
    /// left as the closure and the other process left it; the caller must reconcile.
    /// Also returned when the captured operation is no longer within the probed
    /// window (`ROLLBACK_PROBE_LIMIT` operations), so the range cannot be confirmed
    /// safe to revert.
    SkippedDiverged,
    /// The rollback itself failed — the divergence probe or the `op restore` errored.
    /// The repo may be left mid-transaction; the carried [`Error`] is the cause.
    Failed(Error),
    /// No rollback was attempted: the transaction failed before it captured a
    /// savepoint (e.g. the initial [`op_head`](JjApi::op_head) capture itself failed),
    /// so there was nothing to roll back.
    NotAttempted,
}

impl Rollback {
    /// Whether the repo was returned to (or already at) the captured operation.
    pub fn is_restored(&self) -> bool {
        matches!(self, Rollback::Restored)
    }

    /// The error, when the rollback itself [`Failed`](Rollback::Failed); `None`
    /// otherwise. Reads it off without destructuring the `#[non_exhaustive]` enum.
    pub fn failure(&self) -> Option<&Error> {
        match self {
            Rollback::Failed(err) => Some(err),
            _ => None,
        }
    }
}

impl std::fmt::Display for Rollback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rollback::Restored => f.write_str("rolled back to the captured operation"),
            Rollback::SkippedDiverged => f.write_str(
                "rollback skipped: the operation log diverged (a concurrent jj process \
                 advanced it), so reverting was refused to avoid clobbering that work",
            ),
            Rollback::Failed(err) => write!(f, "rollback failed: {err}"),
            Rollback::NotAttempted => f.write_str("no rollback was attempted"),
        }
    }
}

/// The error [`Jj::transaction`] returns when its closure fails. It preserves the
/// closure's own error in [`cause`](Self::cause) — the same value the previous
/// (rollback-swallowing) `Result<T>` contract returned — and additionally records
/// what the concurrency-safe rollback did in [`rollback`](Self::rollback), so a
/// failed or refused rollback is **visible** to the caller instead of silently
/// dropped (the earlier `let _ = op_restore(..)` discarded it).
///
/// Match [`rollback`](Self::rollback) to distinguish `Restored` / `SkippedDiverged`
/// / `Failed`; call [`into_cause`](Self::into_cause) for a drop-in of the old
/// "closure error only" behavior.
#[derive(Debug)]
#[non_exhaustive]
pub struct TransactionError {
    /// The error the closure returned — the transaction's root cause.
    pub cause: Error,
    /// What the concurrency-safe rollback did in response to `cause`.
    pub rollback: Rollback,
}

impl TransactionError {
    /// The closure's error — the transaction's root cause (what the old `Result<T>`
    /// contract returned).
    pub fn cause(&self) -> &Error {
        &self.cause
    }

    /// What the rollback did — [`Restored`](Rollback::Restored) /
    /// [`SkippedDiverged`](Rollback::SkippedDiverged) / [`Failed`](Rollback::Failed).
    pub fn rollback(&self) -> &Rollback {
        &self.rollback
    }

    /// Consume, returning just the closure's error — the drop-in for code that only
    /// wants the old "closure error" and does not act on the rollback outcome.
    pub fn into_cause(self) -> Error {
        self.cause
    }
}

impl std::fmt::Display for TransactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "transaction failed: {} ({})", self.cause, self.rollback)
    }
}

impl std::error::Error for TransactionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // The closure's error is the root cause; a rollback failure (if any) is
        // reachable structurally through `self.rollback`.
        Some(&self.cause)
    }
}

/// The rollback decision derived from the op-log divergence probe (`rows`, newest
/// first) and the captured pre-operation `pre`. Walking from the current head toward
/// `pre`: a `>= 2`-parent operation seen *before* reaching `pre` is a concurrent
/// "reconcile divergent operations" merge (foreign work) → refuse; reaching `pre`
/// means the range is our own linear work → restore; not finding `pre` within the
/// probed window means the range can't be confirmed safe → refuse (conservative:
/// never blindly revert what we can't verify). `rows` can only be constructed after
/// the safety-probe parser has validated every id/count pair; malformed output fails
/// before this decision can select `Restore`.
enum RollbackPlan {
    Restore,
    SkipDiverged,
}

fn rollback_plan(rows: &[(String, usize)], pre: &str) -> RollbackPlan {
    for (id, parents) in rows {
        if id == pre {
            // Reached the savepoint with only our own single-parent ops in between.
            return RollbackPlan::Restore;
        }
        if *parents >= 2 {
            // A reconcile-divergent-operations merge landed after `pre`: a
            // concurrent process advanced the op log. Do not clobber it.
            return RollbackPlan::SkipDiverged;
        }
    }
    RollbackPlan::SkipDiverged
}

impl<R: ProcessRunner> Jj<R> {
    /// Resolve several workspaces' root paths in one **bounded fan-out** — one
    /// `jj workspace root --name <n>` per name, at most
    /// `WORKSPACE_ROOTS_CONCURRENCY` (8) live at a time — instead of awaiting each in
    /// turn. Per-name `Ok`/`Err` mirrors [`workspace_root`](JjApi::workspace_root)
    /// (a non-zero exit or spawn failure → `Err`); results come back in `names`
    /// order. Runs through this client's own runner, so a `ScriptedRunner` test
    /// drives it hermetically. Inherent (not on the object-safe trait): it's a
    /// throughput shape over the trait method, and the batch primitive isn't a
    /// mockable per-call seam.
    pub async fn workspace_roots(&self, dir: &Path, names: &[String]) -> Vec<Result<PathBuf>> {
        // `--ignore-working-copy`: read-only metadata probe (often on the Drop-cleanup
        // path), so it must not snapshot/lock the working copy (M10).
        let commands = names.iter().map(|n| {
            self.cmd_in(
                dir,
                [
                    "--ignore-working-copy",
                    "workspace",
                    "root",
                    "--name",
                    n.as_str(),
                ],
            )
        });
        // `output_all_bytes` (not `output_all`): a workspace root path need not be
        // valid UTF-8 on Unix, so capture raw stdout and build the `PathBuf` from
        // bytes — a lossy `String` decode would flatten a non-UTF-8 root to `U+FFFD`.
        processkit::output_all_bytes(commands, WORKSPACE_ROOTS_CONCURRENCY, self.core.runner())
            .await
            .into_iter()
            .map(|r| {
                r.and_then(|pr| pr.ensure_success())
                    // Raw bytes → `PathBuf`, lossless on Unix — exact parity with the
                    // single `workspace_root` (both go through `workspace_root_from_bytes`,
                    // which strips only the trailing line terminator jj appends).
                    .map(|pr| parse::workspace_root_from_bytes(pr.stdout()))
            })
            .collect()
    }

    /// Bind this client to `dir`, returning a [`JjAt`] handle whose methods omit
    /// the `dir` argument: `jj.at(dir).status()` runs [`status`](JjApi::status)
    /// against `dir`. The dir-taking [`JjApi`] methods stay on [`Jj`] for driving
    /// many directories (e.g. workspaces) from one client.
    pub fn at<'a>(&'a self, dir: &'a Path) -> JjAt<'a, R> {
        JjAt { jj: self, dir }
    }

    /// Build a repo-scoped `jj` command for the rollback **cleanup** that does
    /// **not** inherit this client's [`default_cancel_on`](Jj::default_cancel_on)
    /// token and carries its own bounded [`ROLLBACK_TIMEOUT`] deadline.
    ///
    /// [`cmd_in`](Self::cmd_in) gap-fills the client's cancel token; overriding it
    /// here with a *fresh, never-fired* token means an already-fired cancellation of
    /// the failed operation cannot also short-circuit the cleanup (the defect the old
    /// `transaction` documented). The explicit timeout gives the cleanup a full fresh
    /// budget even after a cancelled/timed-out main operation. This local metadata
    /// cleanup intentionally does not use the network completion helper: its fresh
    /// token is never fired, and the dedicated deadline keeps rollback independent
    /// of the cancelled mutation.
    fn rollback_cmd_in<I, S>(&self, dir: &Path, args: I) -> processkit::Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.cmd_in(dir, args)
            .cancel_on(CancellationToken::new())
            .timeout(ROLLBACK_TIMEOUT)
    }

    /// The divergence probe: the recent operation log as validated
    /// `(id, parent-count)` pairs (newest first), read on the detached cleanup
    /// context with `--ignore-working-copy` so the *probe itself* records no snapshot
    /// operation. Malformed rows are a parse error rather than a trusted zero count.
    async fn op_log_parents_probe(&self, dir: &Path) -> Result<Vec<(String, usize)>> {
        // Capture the untrimmed ProcessResult: `CliClient::run` strips trailing
        // whitespace and could normalize an extra final field into a valid-looking
        // row before the fail-closed parser sees it.
        let out = self
            .core
            .output_string(self.rollback_cmd_in(
                dir,
                [
                    "op",
                    "log",
                    "--no-graph",
                    "--ignore-working-copy",
                    "--limit",
                    ROLLBACK_PROBE_LIMIT,
                    "-T",
                    parse::OP_PARENTS_TEMPLATE,
                ],
            ))
            .await?
            .ensure_success()?;
        parse::parse_op_parents(out.stdout())
    }

    /// `op restore <op_id>` on the detached cleanup context (see
    /// [`rollback_cmd_in`](Self::rollback_cmd_in)), keeping the flag-like guard the
    /// public [`op_restore`](JjApi::op_restore) applies.
    async fn op_restore_detached(&self, dir: &Path, op_id: &str) -> Result<()> {
        reject_flag_like("operation id", op_id)?;
        self.core
            .run_unit(self.rollback_cmd_in(dir, ["op", "restore", op_id]))
            .await
    }

    /// Roll the repo back to `pre` — an operation id captured with
    /// [`op_head`](JjApi::op_head) **before** a mutation — after that mutation failed.
    /// This is the rollback [`transaction`](Self::transaction) runs, exposed for the
    /// non-closure / FFI callers the transaction docs point at.
    ///
    /// Unlike a bare [`op_restore`](JjApi::op_restore) back to `pre`, it:
    /// - runs every cleanup command on a **fresh cancellation context** with its own
    ///   `ROLLBACK_TIMEOUT` deadline, so a *cancelled or timed-out* mutation does
    ///   not also cancel the cleanup (a fired
    ///   [`default_cancel_on`](Jj::default_cancel_on) token is not inherited);
    /// - **validates the complete divergence probe** before deciding to restore;
    ///   malformed parent-count output is reported as [`Rollback::Failed`] and no
    ///   restore is attempted;
    /// - **detects a concurrent op-log divergence** first — if another jj process
    ///   advanced the operation log between the capture and now (jj records a
    ///   "reconcile divergent operations" merge), reverting to `pre` would silently
    ///   discard that foreign work, so it is **refused** and
    ///   [`Rollback::SkippedDiverged`] is returned instead of clobbering it.
    ///
    /// Never returns `Err`: a failure of the probe or the `op restore` is reported as
    /// [`Rollback::Failed`], so the caller composes the rollback outcome with the
    /// mutation's own error rather than having one mask the other.
    pub async fn rollback_to(&self, dir: &Path, pre: &str) -> Rollback {
        match self.op_log_parents_probe(dir).await {
            Err(err) => Rollback::Failed(err),
            Ok(rows) => match rollback_plan(&rows, pre) {
                RollbackPlan::SkipDiverged => Rollback::SkippedDiverged,
                RollbackPlan::Restore => match self.op_restore_detached(dir, pre).await {
                    Ok(()) => Rollback::Restored,
                    Err(err) => Rollback::Failed(err),
                },
            },
        }
    }

    /// Run a mutation sequence with concurrency-safe op-log rollback: capture the
    /// current operation ([`op_head`](JjApi::op_head)), run `f` with a [`JjAt`] bound
    /// to `dir`, and on `Err` roll the repo back to the captured operation via
    /// [`rollback_to`](Self::rollback_to) — reporting what the rollback did on the
    /// returned [`TransactionError`].
    ///
    /// ```no_run
    /// # async fn demo(jj: &vcs_jj::Jj) -> Result<(), vcs_jj::TransactionError> {
    /// jj.transaction(std::path::Path::new("."), |tx| async move {
    ///     tx.describe("wip").await?;
    ///     tx.new_change("next").await // an Err here rolls back the describe
    /// })
    /// .await?;
    /// # Ok(()) }
    /// ```
    ///
    /// On the closure's `Err`, the returned [`TransactionError`] preserves that error
    /// in [`cause`](TransactionError::cause) **and** carries the
    /// [`rollback`](TransactionError::rollback) outcome — so a failed
    /// ([`Rollback::Failed`]) or refused ([`Rollback::SkippedDiverged`]) rollback is
    /// visible, not swallowed as it was before. Callers wanting only the previous
    /// "closure error" behavior use [`TransactionError::into_cause`].
    ///
    /// Inherent (not on the object-safe trait): the closure parameter is
    /// generic, which `mockall` / trait objects can't express.
    ///
    /// Caveats:
    /// - **Single-actor, but no longer silent about it.** The rollback restores the
    ///   whole repo view to the captured operation, so it is meant for a span *one*
    ///   actor drives. If another jj process advances the op log in the meantime, the
    ///   rollback now **detects** the divergence and **refuses** to revert (returning
    ///   [`Rollback::SkippedDiverged`]) rather than silently reverting that foreign
    ///   work — the caller is told, and must reconcile.
    /// - Rollback runs on `Err` only — **not** on panic or cancellation (a
    ///   dropped future); there is no async `Drop`. Convert panics to `Err`
    ///   inside `f` if you need that safety.
    /// - **A cancelled `f` no longer cancels the rollback.** The cleanup runs on a
    ///   fresh cancellation context with its own deadline (see
    ///   [`rollback_to`](Self::rollback_to)), so a *fired* cancellation of `f` (on a
    ///   client built with [`default_cancel_on`](Jj::default_cancel_on)) does not
    ///   short-circuit the restore.
    /// - If the restore itself fails, the closure's error is still returned as
    ///   [`cause`](TransactionError::cause) and the failure is surfaced as
    ///   [`Rollback::Failed`] (no longer discarded); the repo may be left
    ///   mid-transaction.
    ///
    /// **Non-closure / FFI callers**: the borrowed [`JjAt`] and the `'a`-bound
    /// future this closure form takes don't cross an FFI boundary cleanly, so a
    /// language binding replicates the rollback with the public primitives this
    /// method wraps — capture [`op_head`](JjApi::op_head) before the mutations, run
    /// them (through a [`JjAt`] or the dir-taking methods), then on failure call
    /// [`rollback_to`](Self::rollback_to) with the captured id (it applies the same
    /// cancellation-safe, divergence-checked protocol). [`op_head`](JjApi::op_head)
    /// is on the object-safe [`JjApi`], so the capture also works through
    /// `&dyn JjApi`; `rollback_to` is inherent on [`Jj`].
    pub async fn transaction<'a, T, F, Fut>(
        &'a self,
        dir: &'a Path,
        f: F,
    ) -> std::result::Result<T, TransactionError>
    where
        F: FnOnce(JjAt<'a, R>) -> Fut,
        Fut: Future<Output = Result<T>> + 'a,
    {
        let pre = match self.op_head(dir).await {
            Ok(pre) => pre,
            // The savepoint capture failed before `f` ran, so nothing was mutated
            // and there is nothing to roll back.
            Err(cause) => {
                return Err(TransactionError {
                    cause,
                    rollback: Rollback::NotAttempted,
                });
            }
        };
        match f(self.at(dir)).await {
            Ok(value) => Ok(value),
            Err(cause) => {
                let rollback = self.rollback_to(dir, &pre).await;
                Err(TransactionError { cause, rollback })
            }
        }
    }
}
