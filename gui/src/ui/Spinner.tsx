import { LoaderCircle } from 'lucide-react';

import { Icon } from './Icon';

/**
 * The one spinner.
 *
 * It exists so that "working" is the same glyph at the same speed everywhere: the
 * old tree had antd's `Spin` in the transcript, a hand-written CSS circle in the
 * flash log and a `Loading…` label in the settings drawer -- three different ways
 * to say one thing, one of them a word.
 *
 * `label` has a default rather than being optional-with-no-name, because a spinner
 * no screen reader can describe is a spinner that tells a keyboard user the
 * interface has stopped. Pass the specific thing when the row knows it: "Building",
 * "Flashing".
 *
 * No size or colour of its own: `Icon` already takes both from the text around it.
 */
export function Spinner({
  size,
  label = 'Working',
}: {
  size?: 'sm' | 'md' | 'lg';
  label?: string;
}) {
  return <Icon src={LoaderCircle} spin size={size} label={label} />;
}
