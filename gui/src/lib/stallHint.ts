/**
 * Pure part of ChatView's "nothing is happening" notice: when it appears and
 * what it says. Kept out of the component so the threshold is testable in the
 * node environment (this repo's vitest has no jsdom).
 */

/**
 * Seconds of no visible change before the notice shows. It must stay above
 * the backend's own budget (`STREAM_TIMEOUT` in agent.rs, 120s by default and
 * configurable as `stream_timeout_secs`): a model writing one enormous
 * tool-call argument is silent for exactly that long, and warning at 60s told
 * the user the turn was wedged while the agent was still happily streaming —
 * and still 60s away from failing it.
 */
export const STALL_NOTICE_SECS = 150;

export function shouldShowStallNotice(idleSecs: number): boolean {
  return idleSecs >= STALL_NOTICE_SECS;
}

export function stallNotice(idleSecs: number): {
  message: string;
  description: string;
} {
  return {
    message: `Running with no visible output for ${idleSecs}s.`,
    description:
      'A long tool call or reasoning phase is silent without anything being wrong. ' +
      'The agent gives up on the stream itself and reports the failure — nothing you ' +
      'typed or wrote is lost. Click Stop to end this turn early and retry.',
  };
}
