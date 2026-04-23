// features/updater/UpdateDialog.tsx — Dismissible update notification dialog
//
// Shows when a non-critical update is available. Displays version, release notes,
// download progress, and error/retry states. Uses existing Dialog component.

import { Dialog } from "../../components/ui/Dialog";
import { Button } from "../../components/ui/Button";
import { Spinner } from "../../components/ui/Spinner";
import { useUpdateStore } from "../../stores/updateStore";
import { useUpdater } from "./useUpdater";
import { useI18n } from "../../lib/i18n";

function formatBytes(bytes: number): string {
  if (!bytes || !isFinite(bytes)) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const val = bytes / Math.pow(1024, i);
  return `${val.toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

export function UpdateDialog() {
  const { t } = useI18n();
  const { status, updateInfo, progress, isCritical, error } = useUpdateStore();
  const { downloadAndInstall, dismissUpdate } = useUpdater();

  // Only show for non-critical updates in available/downloading/error states.
  // Critical updates are handled by CriticalUpdateScreen instead.
  const isVisible =
    !isCritical &&
    (status === "available" ||
      status === "downloading" ||
      status === "installing" ||
      status === "error");

  const canDismiss = status === "available";

  const handleClose = () => {
    if (canDismiss) {
      dismissUpdate();
    }
  };

  return (
    <Dialog
      open={isVisible}
      onClose={handleClose}
      title={t("update.available")}
      width="440px"
      className="update-dialog"
    >
      {/* ── Available state: version info + release notes ── */}
      {status === "available" && updateInfo && (
        <>
          <p className="update-version">
            {t("update.newVersion", { version: updateInfo.version })}
          </p>

          {updateInfo.body && (
            <div className="update-release-notes">
              <span className="update-release-notes-label">
                {t("update.releaseNotes")}
              </span>
              <pre className="update-release-notes-body">
                {updateInfo.body.replace(/\[CRITICAL\]\s*/g, "")}
              </pre>
            </div>
          )}

          <div className="dialog-actions">
            <Button variant="ghost" onClick={handleClose}>
              {t("update.later")}
            </Button>
            <Button onClick={() => void downloadAndInstall()}>
              {t("update.updateNow")}
            </Button>
          </div>
        </>
      )}

      {/* ── Downloading state: progress bar ── */}
      {status === "downloading" && (
        <div className="update-progress-container">
          <p className="update-progress-label">{t("update.downloading")}</p>
          <div className="update-progress-bar">
            <div
              className="update-progress-fill"
              style={{ width: `${progress?.percentage ?? 0}%` }}
            />
          </div>
          <p className="update-progress-detail">
            {t("update.progress", {
              percentage: progress?.percentage ?? 0,
              downloaded: formatBytes(progress?.downloaded ?? 0),
              total: progress?.total ? formatBytes(progress.total) : "—",
            })}
          </p>
        </div>
      )}

      {/* ── Installing state: spinner ── */}
      {status === "installing" && (
        <div className="update-progress-container">
          <Spinner size={20} />
          <p className="update-progress-label">{t("update.installing")}</p>
        </div>
      )}

      {/* ── Error state: message + retry ── */}
      {status === "error" && (
        <div className="update-error-container">
          <p className="update-error-title">{t("update.error")}</p>
          {error && <p className="update-error-message">{error}</p>}
          <div className="dialog-actions">
            <Button variant="ghost" onClick={handleClose}>
              {t("update.later")}
            </Button>
            <Button onClick={() => void downloadAndInstall()}>
              {t("update.retry")}
            </Button>
          </div>
        </div>
      )}
    </Dialog>
  );
}
