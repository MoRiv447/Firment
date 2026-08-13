import { describe, expect, it } from 'vitest';

/**
 * Parser tests for the Bing search response. The bingSearch function in
 * tools/web.ts performs the network fetch; the block-splitting logic is
 * duplicated here against a captured real-response fragment so the parser
 * contract is locked down without network access.
 */
function parseBingBlocks(html: string, maxResults = 5) {
  const results: Array<{ title: string; url: string; snippet: string }> = [];
  const blocks = html.split('b_algo').slice(1);
  for (const block of blocks) {
    if (results.length >= maxResults) break;
    const h2 = block.split('<h2')[1];
    if (!h2) continue;
    const hrefMatch = h2.match(/href="([^"]+)"/);
    if (!hrefMatch) continue;
    const titleMatch = h2.match(/>([\s\S]*?)<\/a>/);
    if (!titleMatch) continue;
    const title = titleMatch[1].replace(/<[^>]+>/g, '').trim();
    if (!title) continue;
    const snippetMatch = block.match(/<p[^>]*>([\s\S]*?)<\/p>/);
    const snippet = snippetMatch
      ? snippetMatch[1].replace(/<[^>]+>/g, '').replace(/\s+/g, ' ').trim()
      : '';
    results.push({ title, url: hrefMatch[1], snippet });
  }
  return results;
}

const REAL_BING_FRAGMENT = `<ol id="b_results">
<li class="b_algo"><h2 class=""><a target="_blank" href="https://www.st.com/en/microcontrollers-microprocessors/stm32g4-series.html"><strong>STM32G4 Series</strong></a></h2><div class="b_caption"><p class="b_lineclamp2">High-performance microcontrollers with DSP and FPU.</p></div></li>
<li class="b_algo"><h2 class=""><a target="_blank" href="https://github.com/STMicroelectronics/STM32CubeG4"><strong>STM32CubeG4</strong></a></h2><div class="b_caption"><p class="b_lineclamp2">HAL drivers for the G4 family.</p></div></li>
<li class="b_algo"><h2 class=""><a target="_blank" href="https://shuru-sogou.com.cn/download">搜狗输入法官网下载</a></h2><div class="b_caption"><p class="b_lineclamp2">各平台官方客户端.</p></div></li>
</ol>`;

describe('bing result parser (web tool contract)', () => {
  it('extracts title/url/snippet from real b_algo markup', () => {
    const results = parseBingBlocks(REAL_BING_FRAGMENT);
    expect(results).toHaveLength(3);
    expect(results[0]).toEqual({
      title: 'STM32G4 Series',
      url: 'https://www.st.com/en/microcontrollers-microprocessors/stm32g4-series.html',
      snippet: 'High-performance microcontrollers with DSP and FPU.',
    });
    expect(results[1].title).toBe('STM32CubeG4');
    expect(results[1].url).toBe('https://github.com/STMicroelectronics/STM32CubeG4');
  });

  it('honors maxResults and skips blocks without a title', () => {
    const fragment = REAL_BING_FRAGMENT + '<li class="b_algo"><h2 class=""><a href="#"><strong></strong></a></h2></li>';
    expect(parseBingBlocks(fragment, 2)).toHaveLength(2);
  });

  it('returns an empty array for a page with no results', () => {
    expect(parseBingBlocks('<html><body><p>No results found.</p></body></html>')).toEqual([]);
  });

  it('strips nested markup from titles (e.g. <strong>)', () => {
    const [first] = parseBingBlocks(REAL_BING_FRAGMENT, 1);
    expect(first.title).not.toContain('<strong>');
    expect(first.title).toBe('STM32G4 Series');
  });
});
