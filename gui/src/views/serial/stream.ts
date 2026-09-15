import type { MonitorLine } from '../../types';

/** One rendered line of the terminal: the text, and whether stderr wrote it. */
export interface Block {
  text: string;
  stderr: boolean;
}

/**
 * Turn the monitor's chunks into terminal lines.
 *
 * A serial read does not arrive in lines. The reader hands over whatever the port
 * had, so a chunk usually ends mid-word -- gluing it onto the previous chunk is
 * what makes this read like a terminal instead of like a list of fragments, and it
 * is the one piece of real logic in the serial view. It is here, away from the
 * component, because it is a rule about bytes rather than about React.
 *
 * Two details that are easy to lose:
 *
 *   * a chunk that *does* end with a newline closes the line, so the next chunk
 *     starts a fresh one -- without that, everything after the first newline would
 *     glue onto the same line forever;
 *   * stderr only glues onto stderr. A warning that arrives in the middle of a
 *     sentence is still the warning's own colour.
 */
export function toStream(chunks: MonitorLine[]): Block[] {
  const blocks: Block[] = [];
  let last: Block | null = null;
  for (const chunk of chunks) {
    const stderr = chunk.kind === 'stderr';
    const endsWithNl = chunk.line.endsWith('\n');
    const text = endsWithNl ? chunk.line.slice(0, -1) : chunk.line;
    if (last && last.stderr === stderr && !last.text.endsWith('\n')) {
      last.text += text;
    } else {
      last = { text, stderr };
      blocks.push(last);
    }
    if (endsWithNl) last = null;
  }
  return blocks;
}
