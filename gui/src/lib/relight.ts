/**
 * Which of the sessions a mount snapshot reported are still worth re-lighting.
 *
 * A window reopened mid-turn has an empty reducer, so App asks the backend which sessions have a
 * live turn and dispatches a `turn_start` for each — otherwise the turn is invisible: no spinner,
 * and the input re-enabled. That answer is a snapshot taken an IPC round trip before it lands, and
 * a turn can finish while the question is in flight.
 *
 * The backend already holds a turn's closing notice back until the slot says the turn is over,
 * which makes the snapshot honest about the instant it was taken. It cannot make it honest about
 * the instant it arrives: the reply and a `turn_end` are two separate deliveries to the webview,
 * and the event can be delivered first. Re-lighting after that puts a spinner on a turn that has
 * already closed itself, and no later event exists that could stop it.
 *
 * So the one thing the window knows that the snapshot does not: whether this session's end has
 * already come through. Dropping those is safe in the other direction too, because a turn that is
 * genuinely running cannot have sent its end yet — and if its real `turn_start` did arrive in the
 * meantime, the slot is already lit and this filter changes nothing.
 */
export function relightable(
  reported: readonly string[],
  endedAlready: ReadonlySet<string>,
): string[] {
  return reported.filter((id) => !endedAlready.has(id));
}
