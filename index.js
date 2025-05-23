// Load .env variables in non-production environments
if (process.env.NODE_ENV !== "production") {
    require("dotenv").config();
}

const express = require("express");
const fetch = require("node-fetch");
const axios = require("axios");

const addon = express();
const PORT = process.env.PORT || 13470;

const API_KEY = process.env.API_KEY;

if (API_KEY) {
    console.log("API_KEY is set. All requests will require it as an 'api_key=your_key' URL query parameter.");
}

// Middleware to check for API key if defined
const checkApiKey = (req, res, next) => {
    if (API_KEY) {
        if (req.query.api_key !== API_KEY) {

            console.warn(`Access Denied: Incorrect or missing api_key. Path: ${req.originalUrl}`);
            if (res.socket && !res.socket.destroyed) {
                res.socket.destroy();
            }
            return;
        }
    }
    next();
};

// Apply this middleware to all routes
addon.use(checkApiKey);

// Normalize and validate TORRENTIO_URL
let rawTorrentioUrl = process.env.TORRENTIO_URL || "";
if (rawTorrentioUrl.startsWith("stremio://")) {
    rawTorrentioUrl = rawTorrentioUrl.replace("stremio://", "https://");
}
if (rawTorrentioUrl.endsWith("/manifest.json")) {
    rawTorrentioUrl = rawTorrentioUrl.replace(/\/manifest\.json$/, "");
}
if (!rawTorrentioUrl.startsWith("https://")) {
    console.error("TORRENTIO_URL must be defined and start with https://");
    process.exit(1);
}
const TORRENTIO_URL = rawTorrentioUrl;

// Validate PROXY_SERVER_URL (existence only)
const PROXY_SERVER_URL = process.env.PROXY_SERVER_URL;
if (!PROXY_SERVER_URL) {
    console.error("PROXY_SERVER_URL must be defined");
    process.exit(1);
}

// Respond helper to set common headers for all responses
const respond = (res, data) => {
    res.setHeader("Access-Control-Allow-Origin", "*");
    res.setHeader("Access-Control-Allow-Headers", "*");
    res.setHeader("Content-Type", "application/json");
    res.send(data);
};

// Stremio addon manifest
const MANIFEST = {
    id: "org.custom.torrentio-debrid-proxy",
    version: "1.0.0",
    name: "Torrentio Debrid Proxy",
    description: "Streams via Torrentio with Real-Debrid, proxied through your own server.",
    types: ["movie", "series"],
    resources: ["stream"],
    catalogs: [],
    idPrefixes: ["tt"]
};

// Serve the manifest
addon.get("/manifest.json", (req, res) => {
    respond(res, MANIFEST);
});

// Serve stream metadata, and rewrite RD URLs to point through this proxy
addon.get("/stream/:type/:id.json", async (req, res) => {
    const { type, id } = req.params;
    console.log("Processing stream request (access-checked):", type, id);

    const apiUrl = type === "movie"
        ? `${TORRENTIO_URL}/stream/movie/${id}.json`
        : `${TORRENTIO_URL}/stream/series/${id}.json`;

    try {
        const { data } = await axios.get(apiUrl);

        const streams = (data.streams || []).map(stream => {
            if (!stream.url.includes("/realdebrid/")) return stream;

            let newUrl = stream.url.replace("https://torrentio.strem.fun", PROXY_SERVER_URL);
            
            if (API_KEY) {
                const separator = newUrl.includes('?') ? '&' : '?';
                newUrl += `${separator}api_key=${encodeURIComponent(API_KEY)}`;
            }
            
            return {
                ...stream,
                url: newUrl
            };
        });

        respond(res, { streams });
    } catch (err) {
        console.error("Failed to fetch stream data:", err.message);
        respond(res, { streams: [] });
    }
});

// Cache to store resolved Real-Debrid redirect URLs
const resolvedUrlCache = new Map();

// Route to proxy Real-Debrid streaming URLs
addon.get("/realdebrid/*", (req, res) => {
    const remotePath = req.params[0];
    const rangeHeader = req.headers["range"];
    tryProxyStreamWithFallback(remotePath, rangeHeader, res);
});

async function tryProxyStreamWithFallback(remotePath, rangeHeader, res) {
    const torrentioUrl = `https://torrentio.strem.fun/realdebrid/${remotePath}`;

    const tryFetchAndProxy = async (url, isRetry = false) => {
        const controller = new AbortController();
        const signal = controller.signal;

        try {
            const proxyResp = await fetch(url, {
                headers: rangeHeader ? { Range: rangeHeader } : {},
                signal
            });

            if (proxyResp.status === 404 && !isRetry) {
                console.warn("Cached Real-Debrid URL returned 404. Retrying without cache for path:", remotePath);
                resolvedUrlCache.delete(remotePath);

                const retryUrl = await resolveRDUrl(`https://torrentio.strem.fun/realdebrid/${remotePath}`);
                if (!retryUrl) {
                    if (res.writableEnded === false && !res.destroyed && !(res.socket && res.socket.destroyed)) {
                        return res.status(502).send("Failed to resolve stream URL after retry");
                    }
                    return;
                }

                return tryFetchAndProxy(retryUrl, true);
            }

            if (!proxyResp.ok) {
                console.error(`Remote fetch failed for ${url} with status: ${proxyResp.status}`);
                if (res.writableEnded === false && !res.destroyed && !(res.socket && res.socket.destroyed)) {
                    return res.status(proxyResp.status).send("Failed to fetch stream");
                }
                return;
            }

            res.status(proxyResp.status);
            proxyResp.headers.forEach((value, key) => res.setHeader(key, value));

            const TIMEOUT_MS = 5 * 60 * 1000;
            let timeout = setTimeout(() => {
                console.log(`No activity detected for stream ${remotePath}. Aborting stream.`);
                controller.abort();
            }, TIMEOUT_MS);

            res.on("close", () => {
                console.log(`Client disconnected for stream ${remotePath}.`);
                clearTimeout(timeout);
                controller.abort();
            });

            proxyResp.body.on("data", () => {
                clearTimeout(timeout);
                timeout = setTimeout(() => {
                    console.log(`Idle timeout reached for stream ${remotePath}. Aborting stream.`);
                    controller.abort();
                }, TIMEOUT_MS);
            });

            proxyResp.body.pipe(res);
        } catch (err) {
            if (err.name === "AbortError") {
                console.warn(`Stream aborted for ${remotePath} due to inactivity or disconnect.`);
            } else {
                console.error(`Error while proxying stream ${remotePath}:`, err.message);
            }

            if (!res.headersSent) {
                if (res.writableEnded === false && !res.destroyed && !(res.socket && res.socket.destroyed)) {
                     res.status(502).send("Proxy failed");
                } else {
                    console.warn("Response headers already sent or socket destroyed, cannot send error status for proxy failure.");
                }
            }
        }
    };

    if (resolvedUrlCache.has(remotePath)) {
        const cachedUrl = resolvedUrlCache.get(remotePath);
        console.log("Using cached redirect URL for path:", remotePath, "URL:", cachedUrl);
        return tryFetchAndProxy(cachedUrl);
    }

    const newUrl = await resolveRDUrl(torrentioUrl);
    if (!newUrl) {
        if (res.writableEnded === false && !res.destroyed && !(res.socket && res.socket.destroyed)) {
            return res.status(502).send("Failed to resolve stream URL");
        }
        return;
    }

    return tryFetchAndProxy(newUrl);
}

async function resolveRDUrl(torrentioUrl) {
    try {
        console.log("Resolving redirect for:", torrentioUrl);
        const initialResp = await fetch(torrentioUrl, {
            method: "HEAD",
            redirect: "manual"
        });

        const redirectedUrl = initialResp.headers.get("location");
        if (!redirectedUrl) {
            console.error("Redirect location header not found for:", torrentioUrl);
            return null;
        }

        const cacheKey = torrentioUrl.split("/realdebrid/")[1];
        if (cacheKey) {
             resolvedUrlCache.set(cacheKey, redirectedUrl);
             console.log("Redirect resolved to:", redirectedUrl, "and cached with key:", cacheKey);
        } else {
            console.warn("Could not determine cache key for resolved URL:", redirectedUrl, "from original:", torrentioUrl);
        }
        return redirectedUrl;
    } catch (err) {
        console.error("Error resolving redirect for:", torrentioUrl, err.message);
        return null;
    }
}

addon.listen(PORT, () => {
    console.log(`Addon server is running on port ${PORT}`);
});