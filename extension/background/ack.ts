// A capture is only handed over once Ferric says it has it. Until then the browser's own copy stays
// paused, so a host or daemon that is not really there never costs the user a download.
const waiting = new Map<string, (ok: boolean) => void>()

/** Resolves true when Ferric answers `accepted` for this request id, false on an error or silence. */
export function waitForAck(id: string, timeoutMs: number): Promise<boolean> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      waiting.delete(id)
      resolve(false)
    }, timeoutMs)
    waiting.set(id, (ok) => {
      clearTimeout(timer)
      waiting.delete(id)
      resolve(ok)
    })
  })
}

/** Called for every `accepted` (ok) and `error` (not ok) reply; ids nobody waits for are ignored. */
export function settleAck(id: string | undefined, ok: boolean): void {
  if (id) waiting.get(id)?.(ok)
}
