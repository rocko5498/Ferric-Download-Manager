import test from "node:test"
import assert from "node:assert/strict"
import { waitForAck, settleAck } from "../background/ack.ts"

test("an accepted reply for the id resolves true", async () => {
  const p = waitForAck("a", 1000)
  settleAck("a", true)
  assert.equal(await p, true)
})
test("an error reply resolves false", async () => {
  const p = waitForAck("b", 1000)
  settleAck("b", false)
  assert.equal(await p, false)
})
test("silence resolves false after the timeout, so the browser keeps its download", async () => {
  assert.equal(await waitForAck("c", 20), false)
})
test("a reply for another id, or with none, changes nothing", async () => {
  const p = waitForAck("d", 30)
  settleAck("other", true)
  settleAck(undefined, true)
  assert.equal(await p, false)
})
