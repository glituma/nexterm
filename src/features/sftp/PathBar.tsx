// features/sftp/PathBar.tsx — Always-editable path bar (browser address bar style)
//
// A simple text input that always shows the current path and is always editable.
// Enter navigates, Escape reverts. Navigation buttons (Back/Forward/Home) on the left.

import { useState, useRef, useCallback, useEffect } from "react";
import { useI18n } from "../../lib/i18n";
import type { PaneSource } from "./sftp.types";

export interface PathBarProps {
  source: PaneSource;
  path: string;
  onNavigate: (path: string) => void;
  canGoBack: boolean;
  canGoForward: boolean;
  onGoBack: () => void;
  onGoForward: () => void;
  onGoHome: () => void;
}

export function PathBar({
  source,
  path,
  onNavigate,
  canGoBack,
  canGoForward,
  onGoBack,
  onGoForward,
  onGoHome,
}: PathBarProps) {
  const { t } = useI18n();
  const [inputValue, setInputValue] = useState(path);
  const inputRef = useRef<HTMLInputElement>(null);
  const isFocusedRef = useRef(false);

  // Sync input value when path changes externally — but only when user isn't typing
  useEffect(() => {
    if (!isFocusedRef.current) {
      setInputValue(path);
    }
  }, [path]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Enter") {
        e.preventDefault();
        const trimmed = inputValue.trim();
        if (trimmed && trimmed !== path) {
          onNavigate(trimmed);
        }
        inputRef.current?.blur();
      } else if (e.key === "Escape") {
        e.preventDefault();
        setInputValue(path);
        inputRef.current?.blur();
      }
    },
    [inputValue, path, onNavigate],
  );

  const handleFocus = useCallback(() => {
    isFocusedRef.current = true;
    inputRef.current?.select();
  }, []);

  const handleBlur = useCallback(() => {
    isFocusedRef.current = false;
    // Revert to current path when focus is lost without pressing Enter
    setInputValue(path);
  }, [path]);

  return (
    <div className="sftp-pathbar" data-source={source}>
      {/* Navigation buttons */}
      <div className="sftp-pathbar-nav">
        <button
          className="sftp-nav-btn"
          onClick={onGoBack}
          disabled={!canGoBack}
          title={t("sftp.goBack")}
          aria-label={t("sftp.goBack")}
        >
          {"\u2190"}
        </button>
        <button
          className="sftp-nav-btn"
          onClick={onGoForward}
          disabled={!canGoForward}
          title={t("sftp.goForward")}
          aria-label={t("sftp.goForward")}
        >
          {"\u2192"}
        </button>
        <button
          className="sftp-nav-btn"
          onClick={onGoHome}
          title={t("sftp.goHome")}
          aria-label={t("sftp.goHome")}
        >
          {"\u2302"}
        </button>
      </div>

      {/* Always-editable path input */}
      <input
        ref={inputRef}
        className="sftp-pathbar-input"
        type="text"
        value={inputValue}
        onChange={(e) => setInputValue(e.target.value)}
        onKeyDown={handleKeyDown}
        onFocus={handleFocus}
        onBlur={handleBlur}
        aria-label={t("sftp.navigateTo")}
        autoComplete="off"
        autoCorrect="off"
        autoCapitalize="off"
        spellCheck={false}
        data-form-type="other"
        data-lpignore="true"
      />
    </div>
  );
}
