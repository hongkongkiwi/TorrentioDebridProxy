// Load .env variables in non-production environments
if (process.env.NODE_ENV !== "production") {
    require("dotenv").config();
}

const express = require("express");
const fetch = require("node-fetch");
const axios = require("axios");

const addon = express();
const PORT = process.env.PORT || 13470;

const BASE_URL = process.env.BASE_URL;
const LOCAL_HOST = process.env.LOCAL_HOST;

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
    console.log("Incoming stream request:", type, id);

    const apiUrl = type === "movie"
        ? `${BASE_URL}/stream/movie/${id}.json`
        : `${BASE_URL}/stream/series/${id}.json`;

    try {
        const { data } = await axios.get(apiUrl);

        // Replace RD URLs to be proxied through this server
        const streams = (data.streams || []).map(stream => {
            if (!stream.url.includes("/realdebrid/")) return stream;
            return {
                ...stream,
                url: stream.url.replace("https://torrentio.strem.fun", LOCAL_HOST)
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

/**
 * Attempts to proxy a Real-Debrid URL.
 * If the first attempt fails with a 404 and it's cached, retry without cache.
 */
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

            // If the cached URL returned 404, retry after cache busting
            if (proxyResp.status === 404 && !isRetry) {
                console.warn("Cached Real-Debrid URL returned 404. Retrying without cache.");
                resolvedUrlCache.delete(remotePath);

                const retryUrl = await resolveRDUrl(torrentioUrl);
                if (!retryUrl) {
                    return res.status(502).send("Failed to resolve stream URL after retry");
                }

                return tryFetchAndProxy(retryUrl, true);
            }

            // Handle other fetch errors
            if (!proxyResp.ok) {
                console.error("Remote fetch failed with status:", proxyResp.status);
                return res.status(proxyResp.status).send("Failed to fetch stream");
            }

            // Stream response to client
            res.status(proxyResp.status);
            proxyResp.headers.forEach((value, key) => res.setHeader(key, value));

            const TIMEOUT_MS = 5 * 60 * 1000;
            let timeout = setTimeout(() => {
                console.log("No activity detected. Aborting stream.");
                controller.abort();
            }, TIMEOUT_MS);

            // Abort on client disconnect
            res.on("close", () => {
                console.log("Client disconnected.");
                clearTimeout(timeout);
                controller.abort();
            });

            // Reset timeout on activity
            proxyResp.body.on("data", () => {
                clearTimeout(timeout);
                timeout = setTimeout(() => {
                    console.log("Idle timeout reached. Aborting stream.");
                    controller.abort();
                }, TIMEOUT_MS);
            });

            proxyResp.body.pipe(res);
        } catch (err) {
            if (err.name === "AbortError") {
                console.warn("Stream aborted due to inactivity or disconnect.");
            } else {
                console.error("Error while proxying stream:", err.message);
            }

            if (!res.headersSent) {
                res.status(502).send("Proxy failed");
            }
        }
    };

    // If URL is cached, use it
    if (resolvedUrlCache.has(remotePath)) {
        const cachedUrl = resolvedUrlCache.get(remotePath);
        console.log("Using cached redirect URL:", cachedUrl);
        return tryFetchAndProxy(cachedUrl);
    }

    // Resolve new redirect URL
    const newUrl = await resolveRDUrl(torrentioUrl);
    if (!newUrl) return res.status(502).send("Failed to resolve stream URL");

    return tryFetchAndProxy(newUrl);
}

/**
 * Resolves the final Real-Debrid URL by performing a HEAD request
 * and storing the redirect in cache.
 */
async function resolveRDUrl(torrentioUrl) {
    try {
        console.log("Resolving redirect for:", torrentioUrl);
        const initialResp = await fetch(torrentioUrl, {
            method: "HEAD",
            redirect: "manual"
        });

        const redirectedUrl = initialResp.headers.get("location");
        if (!redirectedUrl) {
            console.error("Redirect location header not found.");
            return null;
        }

        // Cache the resolved URL by extracting the path key
        resolvedUrlCache.set(torrentioUrl.split("/realdebrid/")[1], redirectedUrl);
        console.log("Redirect resolved to:", redirectedUrl);
        return redirectedUrl;
    } catch (err) {
        console.error("Error resolving redirect:", err.message);
        return null;
    }
}

// Start the addon server
addon.listen(PORT, () => {
    console.log(`Addon server is running on port ${PORT}`);
});
