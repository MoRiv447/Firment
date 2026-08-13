import { lookup } from 'node:dns/promises';

export interface SearchResult {
  title: string;
  url: string;
  snippet: string;
}

/** IPv4 ranges that must never be fetched (private/link-local/metadata/etc). */
function isPrivateIpv4(ip: string): boolean {
  const octets = ip.split('.').map(Number);
  if (octets.length !== 4 || octets.some((o) => Number.isNaN(o))) return false;
  const [a, b] = octets;
  return (
    a === 0 ||
    a === 10 ||
    a === 127 ||
    (a === 100 && b >= 64 && b <= 127) || // CGNAT 100.64/10
    (a === 169 && b === 254) || // link-local
    (a === 172 && b >= 16 && b <= 31) || // 172.16/12
    (a === 192 && b === 168) || // 192.168/16
    a >= 224 // multicast + reserved
  );
}

/** IPv6 ranges that must never be fetched. */
function isPrivateIpv6(ip: string): boolean {
  const h = ip.toLowerCase();
  if (h === '::1' || h === '::') return true; // loopback / unspecified
  // IPv4-mapped (::ffff:1.2.3.4) and IPv4-compatible (::1.2.3.4) forms embed
  // a real IPv4 address — validate it with the IPv4 rules. Without this, a
  // literal like [::ffff:127.0.0.1] would connect to 127.0.0.1 (verified:
  // Node fetch honors the mapped form).
  const mapped = h.match(/^::(?:ffff:)?(\d+\.\d+\.\d+\.\d+)$/);
  if (mapped) return isPrivateIpv4(mapped[1]);
  if (h.startsWith('fc') || h.startsWith('fd')) return true; // ULA fc00::/7
  if (h.startsWith('fe8') || h.startsWith('fe9') || h.startsWith('fea') || h.startsWith('feb')) {
    return true; // link-local fe80::/10
  }
  if (h.startsWith('2001:db8')) return true; // documentation
  return false;
}

function isPrivateIp(ip: string): boolean {
  return ip.includes(':') ? isPrivateIpv6(ip) : isPrivateIpv4(ip);
}

/**
 * Resolve a hostname and reject it if ANY resolved address is
 * private/loopback/metadata. This closes the DNS-rebinding hole that a
 * hostname-string check leaves open (a public domain resolving to
 * 127.0.0.1 must never be fetched).
 */
async function assertPublicHost(hostname: string): Promise<void> {
  const h = hostname.toLowerCase().replace(/^\[|\]$/g, '');
  if (h === 'localhost') {
    throw new Error(`Blocked host (loopback): ${hostname}`);
  }
  // If it is already an IP literal, check it directly; otherwise resolve.
  const ipLiteral = h.includes(':') || /^\d+\.\d+\.\d+\.\d+$/.test(h);
  const addresses = ipLiteral ? [h] : await lookup(h, { all: true }).then((r) => r.map((x) => x.address));
  for (const addr of addresses) {
    if (isPrivateIp(addr)) {
      throw new Error(`Blocked host (resolves to private/loopback/metadata IP ${addr}): ${hostname}`);
    }
  }
}

/**
 * Validate a URL for outbound fetches: http/https only, host resolves to
 * a public IP. DNS resolution happens at check time (inside the request
 * path), which is the standard mitigation for DNS rebinding in Node.
 */
async function assertSafeUrl(raw: string): Promise<URL> {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw new Error(`Invalid URL: ${raw}`);
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    throw new Error(`Protocol not allowed: ${url.protocol}`);
  }
  await assertPublicHost(url.hostname);
  return url;
}

/**
 * Fetch with safe redirect handling: follow at most MAX_REDIRECTS hops and
 * re-validate every hop URL (DNS + protocol), so a redirect to a private
 * address cannot smuggle the request into an internal network.
 */
async function fetchWithSafeRedirect(
  url: URL,
  init: RequestInit,
  maxRedirects = 5,
): Promise<Response> {
  let current = url;
  for (let hop = 0; ; hop++) {
    const res = await fetch(current, { ...init, redirect: 'manual' });
    if (res.status >= 300 && res.status < 400) {
      const location = res.headers.get('location');
      res.body?.cancel();
      if (!location || hop >= maxRedirects) {
        throw new Error(`Redirect chain too long or missing Location (${res.status})`);
      }
      current = await assertSafeUrl(new URL(location, current).toString());
      continue;
    }
    return res;
  }
}

export async function webSearch(query: string, maxResults = 5, provider = 'duckduckgo'): Promise<SearchResult[]> {
  if (provider === 'bing') {
    return bingSearch(query, maxResults);
  }
  if (provider !== 'duckduckgo') {
    throw new Error(`Web search provider "${provider}" is not supported in web mode (only duckduckgo / bing).`);
  }
  return duckduckgoSearch(query, maxResults);
}

/**
 * Bing (cn.bing.com) — no API key, reachable from mainland China where
 * DuckDuckGo is unreliable. Mirrors the core CLI parser: split on
 * `b_algo`, take the first href inside the `<h2>` (loose match — attributes
 * vary), strip tags from the title, and read the first `<p>` as snippet.
 */
async function bingSearch(query: string, maxResults: number): Promise<SearchResult[]> {
  const url = await assertSafeUrl(
    `https://cn.bing.com/search?q=${encodeURIComponent(query)}&count=${maxResults}&mkt=zh-CN`,
  );
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 15000);
  let html: string;
  try {
    const res = await fetchWithSafeRedirect(url, {
      headers: { 'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36' },
      signal: controller.signal,
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    html = await res.text();
  } catch (err: any) {
    throw new Error(`Web search failed: ${err?.message || 'timeout or network error'}`);
  } finally {
    clearTimeout(timer);
  }

  const results: SearchResult[] = [];
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
  if (results.length === 0 && !/no results/i.test(html)) {
    throw new Error(
      'Web search failed: bing returned a page without results or a "no results" message (rate-limited or blocked from this network). Try web_fetch on a known URL instead.',
    );
  }
  return results;
}

async function duckduckgoSearch(query: string, maxResults: number): Promise<SearchResult[]> {
  const encodedQuery = encodeURIComponent(query);
  const url = await assertSafeUrl(`https://html.duckduckgo.com/html/?q=${encodedQuery}`);
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 15000);
  let html: string;
  try {
    const res = await fetchWithSafeRedirect(url, {
      headers: { 'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)' },
      signal: controller.signal,
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
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
  const url = await assertSafeUrl(rawUrl);
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 20000);
  let response: Response;
  try {
    response = await fetchWithSafeRedirect(url, {
      headers: { 'User-Agent': 'Firment/0.4' },
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
