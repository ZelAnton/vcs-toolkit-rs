# vcs-process

Launch child processes inside an OS **job** so the whole process tree dies with
the parent — no orphaned subprocesses left behind. Part of the
[vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace; the
`vcs-git`, `vcs-jj`, and `vcs-github` wrappers run every command through it.

| Platform | Mechanism |
|---|---|
| Windows | [Job Object](https://learn.microsoft.com/windows/win32/procthread/job-objects) with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` |
| Linux | [cgroup v2](https://docs.kernel.org/admin-guide/cgroup-v2.html) `cgroup.kill`, falling back to a POSIX process group when no writable cgroup is available |
| other | plain spawn, no containment |

```rust
// One-shot helper (async): spawn, capture stdout, then kill any stray
// descendants. Returns the structured `CommandError` on a non-zero exit.
let out = vcs_process::run("git", ["status", "--short"]).await?;

// Or keep a job around and spawn several processes into it. `Job::spawn` takes a
// `tokio::process::Command`; use the `Exec` builder for cwd/env/stdin/timeouts.
let job = vcs_process::Job::new()?;
let mut cmd = tokio::process::Command::new("long-running-tool");
let mut child = job.spawn(&mut cmd)?;
// ... dropping `job` kills child + every descendant (kill-on-close).
```

For long-running commands, stream instead of buffering to completion — stdout
arrives as it is produced, stdin is written incrementally, and stderr is drained
for you in the background:

```rust
let mut s = vcs_process::Exec::new("git").args(["log"]).stream().await?;
while let Some(line) = s.next_line().await? {
    // each line arrives as git emits it, before the process exits
}
let (status, stderr) = s.finish().await?;
```

v1 guarantees **kill-on-close**: terminating or dropping the [`Job`] tears down
the whole tree. Resource limits are intentionally out of scope for now.

## License

MIT
