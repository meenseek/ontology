use crate::domain::Error;
use std::time::Duration;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

async fn read_limited<R: AsyncRead + Unpin>(reader: R, limit: usize) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    reader
        .take(
            u64::try_from(limit)
                .map_err(|_| Error::Limit)?
                .checked_add(1)
                .ok_or(Error::Limit)?,
        )
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| Error::Import)?;
    if bytes.len() > limit {
        return Err(Error::Limit);
    }
    Ok(bytes)
}
pub(crate) async fn run_bounded(
    mut command: Command,
    deadline: Duration,
    limit: usize,
) -> Result<Vec<u8>, Error> {
    let mut child = command.spawn().map_err(|_| Error::Import)?;
    let stdout = child.stdout.take().ok_or(Error::Import)?;
    let stderr = child.stderr.take().ok_or(Error::Import)?;
    let result = tokio::time::timeout(deadline, async {
        let (out, _, status) = tokio::try_join!(
            read_limited(stdout, limit),
            read_limited(stderr, 4096),
            async { child.wait().await.map_err(|_| Error::Import) }
        )?;
        if !status.success() {
            return Err(Error::Import);
        }
        Ok(out)
    })
    .await;
    match result {
        Ok(Ok(bytes)) => Ok(bytes),
        other => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            match other {
                Ok(Err(error)) => Err(error),
                _ => Err(Error::Limit),
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    #[tokio::test]
    async fn subprocess_limits() {
        let mut large = Command::new("/usr/bin/yes");
        large
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        assert_eq!(
            run_bounded(large, Duration::from_secs(1), 100).await,
            Err(Error::Limit)
        );
        let mut slow = Command::new("/bin/sleep");
        slow.arg("2")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let start = std::time::Instant::now();
        assert_eq!(
            run_bounded(slow, Duration::from_millis(30), 100).await,
            Err(Error::Limit)
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
