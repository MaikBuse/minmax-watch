// Keeps the app openable when the LAN server is off.
//
// Everything the draft screen needs is in the bundle — the dataset is compiled
// into the wasm — so a cached shell is a fully working single-player app, not a
// degraded one. Only the sync socket and the match log need the server.
//
// Strategy is network-first with a cache fallback: on a healthy LAN the newest
// build always wins, and the cache only matters when the server is unreachable.
// Cache-first would be faster by a few milliseconds but would serve a stale
// build after every `just build-web`, which is a much worse trade.

const CACHE = 'minmax-v1';

self.addEventListener('install', event => {
    // Take over immediately rather than waiting for every tab to close.
    self.skipWaiting();
    // Only the shell is precached. The hero portraits and map thumbnails are
    // ~2 MB, and pulling all of them here would slow the very first load for
    // artwork the fetch handler below caches anyway the moment it is shown —
    // which, on a screen that lists suggestions as you type, is almost at once.
    event.waitUntil(caches.open(CACHE).then(cache => cache.add('/')));
});

self.addEventListener('activate', event => {
    event.waitUntil(
        caches
            .keys()
            .then(keys => Promise.all(keys.filter(k => k !== CACHE).map(k => caches.delete(k))))
            .then(() => self.clients.claim())
    );
});

self.addEventListener('fetch', event => {
    const request = event.request;
    if (request.method !== 'GET') return;

    const url = new URL(request.url);
    // Never cache the API or the socket: stale draft state would be worse than
    // no draft state.
    if (url.pathname.startsWith('/api/') || url.pathname.startsWith('/ws/')) return;

    event.respondWith(
        fetch(request)
            .then(response => {
                if (response && response.ok) {
                    const copy = response.clone();
                    caches.open(CACHE).then(cache => cache.put(request, copy));
                }
                return response;
            })
            .catch(() =>
                caches.match(request).then(hit => hit || caches.match('/'))
            )
    );
});
