# Protocol research (Tasks 141–148)

| ID | Protocol | Status | Notes |
|----|----------|--------|-------|
| 141 | ed2k / eMule | Research scaffold | Not implemented; legal/abuse risk; defer post-v1 |
| 142 | Magnet → HTTP web seeds | Partial | Torrents use librqbit; HTTP multi-source exists |
| 143 | youtube-dl URL API | Done-ish | `/media/extract` + OpenAPI |
| 144 | RSS/Atom watcher | Scaffold | `rss_watch` + `/rss/feeds` |
| 145 | Browser URL monitoring | Partial | Extension + CDP; no-extension OS hooks deferred |
| 146 | IPFS | Research | Out of v1; use gateway HTTP if needed |
| 147 | Torrent web seeds | Partial | Depends on librqbit webseed support |
| 148 | Direct Connect / ADC | Research flag | Not planned for v1 |

## Decision
Ship HTTP/SFTP/WebDAV/FTP/torrent first. ed2k/IPFS/ADC remain research-only until demand + legal review.
