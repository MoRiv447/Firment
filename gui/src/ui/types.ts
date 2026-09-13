/**
 * The prop vocabulary every primitive shares.
 *
 * One file so a size or a tier cannot mean two different things in two
 * components, and so the union is what a `data-*` attribute is checked against
 * at compile time -- a typo in a `data-tier` value would otherwise just be a
 * style that silently never matches.
 */

/** Three heights, all of them a token: 28 / 36 / 40. */
export type Size = 'sm' | 'md' | 'lg';

/**
 * How much a button announces itself.
 *
 * `primary` is the acid fill and the only one allowed the slanted edge;
 * `secondary` is a hairline box; `ghost` is text that happens to be clickable;
 * `danger` is the removed-diff family, so "this deletes something" and "this
 * line went away" read as the same colour.
 */
export type Tier = 'primary' | 'secondary' | 'ghost' | 'danger';

/**
 * What a chip is reporting. Mirrors `StatusKind` in the old token layer, and
 * deliberately re-declared here: `styles/tokens.ts` is deleted with antd, and a
 * primitive must not depend on a file that is on its way out.
 *
 * `neutral` is "no judgement" -- configured but not connected, usage unknown --
 * and must not read as either healthy or broken.
 */
export type ChipStatus = 'ok' | 'failed' | 'running' | 'attention' | 'neutral';

/** Which side of its anchor a floating panel lands on, after `place()` flips it. */
export type Side = 'top' | 'bottom' | 'left' | 'right';

/** How a panel lines up with the anchor along the cross axis. */
export type Align = 'start' | 'center' | 'end';
