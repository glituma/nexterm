// features/updater/CriticalUpdateScreen.tsx — Full-screen blocking overlay for critical updates
//
// Follows the VaultScreen pattern: covers entire viewport, no dismiss option.
// Only action is "Update Now". On error, shows "Retry" (still can't dismiss).

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

export function CriticalUpdateScreen() {
  const { t } = useI18n();
  const { status, updateInfo, progress, isCritical, error } = useUpdateStore();
  const { downloadAndInstall } = useUpdater();

  // Only render for critical updates in actionable states
  if (!isCritical) return null;
  if (status !== "available" && status !== "downloading" && status !== "installing" && status !== "error") return null;

  return (
    <div className="critical-update-overlay">
      <div className="critical-update-card">
        {/* Warning icon */}
        <div className="critical-update-icon">
          <svg
            width="32"
            height="32"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
            <line x1="12" y1="9" x2="12" y2="13" />
            <line x1="12" y1="17" x2="12.01" y2="17" />
          </svg>
        </div>

        <h1 className="critical-update-title">{t("update.critical")}</h1>
        <p className="critical-update-message">{t("update.criticalMessage")}</p>

        {/* ── Available state: version + update button ── */}
        {status === "available" && updateInfo && (
          <>
            <p className="critical-update-version">
              {t("update.newVersion", { version: updateInfo.version })}
            </p>
            <Button
              onClick={() => void downloadAndInstall()}
              style={{ width: "100%", justifyContent: "center", marginTop: 8 }}
            >
              {t("update.updateNow")}
            </Button>
          </>
        )}

        {/* ── Downloading state: progress bar ── */}
        {status === "downloading" && (
          <div className="critical-update-progress">
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
          <div className="critical-update-progress">
            <Spinner size={20} />
            <p className="update-progress-label">{t("update.installing")}</p>
          </div>
        )}

        {/* ── Error state: message + retry (no dismiss) ── */}
        {status === "error" && (
          <>
            <div className="critical-update-error">
              <p className="update-error-title">{t("update.error")}</p>
              {error && <p className="update-error-message">{error}</p>}
            </div>
            <Button
              onClick={() => void downloadAndInstall()}
              style={{ width: "100%", justifyContent: "center", marginTop: 8 }}
            >
              {t("update.retry")}
            </Button>
          </>
        )}
      </div>
    </div>
  );
}
