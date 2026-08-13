export interface SearchResult {
  title: string;
  url: string;
  snippet: string;
}

/** Private / link-local / metadata IP ranges that must never be fetched. */
const BLOCKED_IPS = [
  /^127\./, /^10\./, /^192\.168\./, /^172\.(1[6-9]|2\d|3[01])\./,
  /^169\.254\./, /^0\./, /^100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\./, // CGNAT
  /^::1$/, /^fc/, /^fd/, /^fe80:/, /^::/, /^2001:db8:/,
];

function isPrivateHost(host: string): boolean {
  const h = host.toLowerCase().replace(/^\[|\]$/g, '');
  if (h === 'localhost') return true;
  if (BLOCKED_IPS.some((re) => re.test(h))) return true;
  return false;
}

/** Validate a URL for outbound fetches: http/https only, no private hosts. */
function assertSafeUrl(raw: string): URL {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw new Error(`Invalid URL: ${raw}`);
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    throw new Error(`Protocol not allowed: ${url.protocol}`);
  }
  if (isPrivateHost(url.hostname)) {
    throw new Error(`Blocked host (private/loopback/metadata): ${url.hostname}`);
  }
  return url;
}

export async function webSearch(query: string, maxResults = 5, provider = 'duckduckgo'): Promise<SearchResult[]> {
  if (provider !== 'duckduckgo') {
    throw new Error(`Web search provider "${provider}" is not supported in web mode (only duckduckgo).`);
  }
  return duckduckgoSearch(query, maxResults);
}

async function duckduckgoSearch(query: string, maxResults: number): Promise<SearchResult[]> {
  const encodedQuery = encodeURIComponent(query);
  const url = assertSafeUrl(`https://html.duckduckgo.com/html/?q=${encodedQuery}`);
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 15000);
  let html: string;
  try {
    const res = await fetch(url, {
      headers: { 'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)' },
      signal: controller.signal,
    });
    html = await res.text();
  } catch (err: any) {
    throw new Error(`Web search failed: ${err?.message || 'timeout or network error'}`);
  } finally {
    clearTimeout(timer);
  }

  const results: SearchResult[] = [];
  // Split into per-result blocks anchored on the title link
  const blocks = html.split('class="result__a"').slice(1);

  for (const block of blocks) {
    if (results.length >= maxResults) break;

    const titleMatch = block.match(/>([^<]+)<\/a>/);
    const hrefMatch = block.match(/href="([^"]+)"/);
    const snippetMatch = block.match(/class="result__snippet"[^>]*>([\s\S]*?)<\/a>/);

    if (titleMatch && hrefMatch) {
      const snippet = snippetMatch
        ? snippetMatch[1].replace(/<[^>]+>/g, '').replace(/\s+/g, ' ').trim()
        : '';
      results.push({
        title: titleMatch[1].replace(/<[^>]+>/g, '').trim(),
        url: extractRealUrl(hrefMatch[1]),
        snippet,
      });
    }
  }

  return results;
}

function extractRealUrl(href: string): string {
  const match = href.match(/uddg=([^&]+)/);
  return match ? decodeURIComponent(match[1]) : href;
}

export async function webFetch(rawUrl: string): Promise<string> {
  const url = assertSafeUrl(rawUrl);
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 20000);
  let response: Response;
  try {
    response = await fetch(url, {
      headers: { 'User-Agent': 'Firment/0.4' },
      redirect: 'follow',
      signal: controller.signal,
    });
  } catch (err: any) {
    clearTimeout(timer);
    throw new Error(`Fetch failed: ${err?.message || 'timeout or network error'}`);
  }
  clearTimeout(timer);
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  const html = await response.text();
  return htmlToText(html);
}

// Convert raw HTML into a compact, readable text blob for the model context.
function htmlToText(html: string): string {
  let text = html.replace(/<script[\s\S]*?<\/script>/gi, ' ');
  text = text.replace(/<style[\s\S]*?<\/style>/gi, ' ');
  text = text.replace(/<head[\s\S]*?<\/head>/gi, ' ');
  text = text.replace(/<nav[\s\S]*?<\/nav>/gi, ' ');
  text = text.replace(/<footer[\s\S]*?<\/footer>/gi, ' ');
  text = text.replace(/<[^>]+>/g, ' ');
  text = text
    .replace(/&nbsp;/g, ' ')
    .replace(/&amp;/g, '&')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&apos;/g, "'");
  text = text.replace(/\s+/g, ' ').trim();
  if (text.length > 20000) text = text.slice(0, 20000) + ' …[truncated]';
  return text;
}
