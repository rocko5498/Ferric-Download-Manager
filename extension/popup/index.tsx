import "../firefox-compat"
import "webextension-polyfill"
import { useEffect, useState } from "react"
import { type CapturePolicy, loadCapturePolicy, saveCapturePolicy } from "../capture-policy"

type Status = {
  daemonConnected: boolean
  lastError: string | null
}

const initialStatus: Status = {
  daemonConnected: false,
  lastError: null
}

type HistoryEntry = {
  id: string
  url: string
  pageUrl?: string
  mimeType?: string
  kind: string
  observedAt: number
}

export default function Popup() {
  const [status, setStatus] = useState<Status>(initialStatus)
  const [policy, setPolicy] = useState<CapturePolicy | null>(null)
  const [saving, setSaving] = useState(false)
  const [history, setHistory] = useState<HistoryEntry[]>([])
  const [domainMode, setDomainMode] = useState<"default" | "all" | "none" | "filter">("default")
  const [domainHost, setDomainHost] = useState("")

  useEffect(() => {
    let active = true
    const refresh = () => {
      void chrome.runtime.sendMessage({ type: "get_status" }).then((response?: Status) => {
        if (active && response) {
          setStatus(response)
        }
      }, () => undefined)
    }

    refresh()
    const timer = window.setInterval(refresh, 1500)
    return () => {
      active = false
      window.clearInterval(timer)
    }
  }, [])

  useEffect(() => {
    let active = true
    loadCapturePolicy().then((loaded) => {
      if (active) {
        setPolicy(loaded)
      }
    })
    void chrome.runtime.sendMessage({ type: "get_media_history" }).then((response?: { entries?: HistoryEntry[] }) => {
      if (active && response?.entries) {
        setHistory(response.entries.slice(0, 12))
      }
    }, () => undefined)
    void chrome.tabs.query({ active: true, currentWindow: true }).then((tabs) => {
      const host = tabs[0]?.url ? safeHost(tabs[0].url) : ""
      if (active) setDomainHost(host)
    })
    return () => {
      active = false
    }
  }, [])

  const openFerric = () => {
    chrome.runtime.sendMessage({ type: "open_ferric" })
  }

  const persistPolicy = async () => {
    if (!policy) {
      return
    }
    setSaving(true)
    try {
      await saveCapturePolicy(policy)
      // Push the updated policy to the daemon so desktop settings stay in sync.
      chrome.runtime.sendMessage({ type: "sync_policy_to_daemon" })
    } finally {
      setSaving(false)
    }
  }

  return (
    <main style={styles.shell}>
      <header style={styles.header}>
        <div>
          <h1 style={styles.title}>Ferric</h1>
          <p style={styles.subtitle}>Download capture</p>
        </div>
        <span
          aria-label={status.daemonConnected ? "Daemon connected" : "Daemon disconnected"}
          style={{
            ...styles.dot,
            background: status.daemonConnected ? "#18a058" : "#d03050"
          }}
        />
      </header>

      <section style={styles.row}>
        <span style={styles.label}>Daemon</span>
        <strong>{status.daemonConnected ? "Connected" : "Not running"}</strong>
      </section>

      {status.lastError ? <p style={styles.error}>{status.lastError}</p> : null}

      {policy ? (
        <section style={styles.policy}>
          <label style={styles.checkRow}>
            <input
              checked={policy.enabled}
              type="checkbox"
              onChange={(event) => setPolicy({ ...policy, enabled: event.currentTarget.checked })}
            />
            Capture browser downloads
          </label>

          <label style={styles.field}>
            <span style={styles.label}>Min size KB</span>
            <input
              style={styles.input}
              min={0}
              type="number"
              value={Math.round(policy.minCaptureBytes / 1024)}
              onChange={(event) =>
                setPolicy({ ...policy, minCaptureBytes: Math.max(0, Number(event.currentTarget.value) || 0) * 1024 })
              }
            />
          </label>

          <label style={styles.field}>
            <span style={styles.label}>Extensions</span>
            <textarea
              style={styles.textarea}
              value={policy.extensions.join(" ")}
              onChange={(event) =>
                setPolicy({
                  ...policy,
                  extensions: event.currentTarget.value
                    .split(/[\s,;]+/)
                    .map((value) => value.trim().replace(/^\./, "").toLowerCase())
                    .filter(Boolean)
                })
              }
            />
          </label>

          <label style={styles.field}>
            <span style={styles.label}>Exclusions</span>
            <textarea
              style={styles.textarea}
              value={policy.excludeUrlPatterns.join("\n")}
              onChange={(event) =>
                setPolicy({
                  ...policy,
                  excludeUrlPatterns: event.currentTarget.value
                    .split(/\n+/)
                    .map((value) => value.trim())
                    .filter(Boolean)
                })
              }
            />
          </label>

          <label style={styles.field}>
            <span style={styles.label}>This site ({domainHost || "—"})</span>
            <select
              style={styles.input}
              value={domainMode}
              onChange={(event) => {
                const mode = event.currentTarget.value as typeof domainMode
                setDomainMode(mode)
                if (!policy || !domainHost) return
                setPolicy({
                  ...policy,
                  domainPolicies: {
                    ...(policy.domainPolicies ?? {}),
                    [domainHost]: mode
                  }
                })
              }}
            >
              <option value="default">Default filter</option>
              <option value="all">Capture all</option>
              <option value="none">Capture none</option>
              <option value="filter">Use filter only</option>
            </select>
          </label>

          <button style={styles.secondaryButton} disabled={saving} onClick={persistPolicy} type="button">
            {saving ? "Saving..." : "Save capture policy"}
          </button>
        </section>
      ) : null}

      {history.length > 0 ? (
        <section style={styles.policy}>
          <div style={styles.label}>Media seen (this session)</div>
          <ul style={{ margin: "8px 0 0", padding: 0, listStyle: "none", maxHeight: 140, overflow: "auto" }}>
            {history.map((entry) => (
              <li key={entry.id} style={{ marginBottom: 6, fontSize: 11, wordBreak: "break-all" }}>
                <div style={{ color: "#5f6c7b" }}>{entry.kind} · {entry.mimeType ?? "media"}</div>
                <div>{entry.url.slice(0, 90)}</div>
                <button
                  type="button"
                  style={{ ...styles.secondaryButton, marginTop: 4, padding: "4px 8px" }}
                  onClick={() => {
                    chrome.runtime.sendMessage({
                      type: "grab_media_history",
                      url: entry.url,
                      referrer: entry.pageUrl
                    })
                  }}
                >
                  Grab
                </button>
              </li>
            ))}
          </ul>
          <button
            type="button"
            style={styles.secondaryButton}
            onClick={() => {
              void chrome.runtime.sendMessage({ type: "clear_media_history" }).then(() => setHistory([]), () => setHistory([]))
            }}
          >
            Clear history
          </button>
        </section>
      ) : null}

      <button style={styles.button} onClick={openFerric} type="button">
        Open Ferric
      </button>
    </main>
  )
}

function safeHost(url: string): string {
  try {
    return new URL(url).hostname
  } catch {
    return ""
  }
}

const styles = {
  shell: {
    width: 280,
    padding: 16,
    color: "#17202a",
    fontFamily:
      'Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'
  },
  header: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    marginBottom: 16
  },
  title: {
    margin: 0,
    fontSize: 20,
    lineHeight: "24px"
  },
  subtitle: {
    margin: "2px 0 0",
    color: "#5f6c7b",
    fontSize: 12
  },
  dot: {
    width: 10,
    height: 10,
    borderRadius: 999,
    boxShadow: "0 0 0 3px rgba(0, 0, 0, 0.06)"
  },
  row: {
    display: "flex",
    justifyContent: "space-between",
    alignItems: "center",
    padding: "10px 0",
    borderTop: "1px solid #e5e8ec",
    fontSize: 14
  },
  label: {
    color: "#5f6c7b"
  },
  error: {
    margin: "10px 0 0",
    color: "#b42318",
    fontSize: 12,
    lineHeight: "16px"
  },
  policy: {
    borderTop: "1px solid #e5e8ec",
    marginTop: 12,
    paddingTop: 12
  },
  checkRow: {
    display: "flex",
    alignItems: "center",
    gap: 8,
    fontSize: 13,
    fontWeight: 700
  },
  field: {
    display: "grid",
    gap: 5,
    marginTop: 10,
    fontSize: 12
  },
  input: {
    border: "1px solid #d0d7de",
    borderRadius: 6,
    padding: "7px 8px"
  },
  textarea: {
    minHeight: 54,
    resize: "vertical",
    border: "1px solid #d0d7de",
    borderRadius: 6,
    fontFamily: "Consolas, monospace",
    fontSize: 11,
    padding: 8
  },
  secondaryButton: {
    width: "100%",
    marginTop: 10,
    border: "1px solid #d0d7de",
    borderRadius: 6,
    background: "#f6f8fa",
    color: "#17202a",
    cursor: "pointer",
    fontWeight: 700,
    padding: "8px 10px"
  },
  button: {
    width: "100%",
    marginTop: 14,
    border: 0,
    borderRadius: 6,
    background: "#1f6feb",
    color: "#fff",
    cursor: "pointer",
    fontWeight: 700,
    padding: "10px 12px"
  }
} satisfies Record<string, React.CSSProperties>
