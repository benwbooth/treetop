.pragma library

function bytes(n, digits) {
    n = Number(n) || 0
    const u = ["B", "K", "M", "G", "T", "P"]
    let i = 0
    while (Math.abs(n) >= 1024 && i < u.length - 1) { n /= 1024; i++ }
    if (i === 0) return n.toFixed(0) + "B"
    const d = digits === undefined ? (n < 10 ? 1 : 0) : digits
    return n.toFixed(d) + u[i]
}

function rate(n) {
    return bytes(n) + "/s"
}

function pct(n) {
    n = Number(n) || 0
    return (n >= 99.95 ? n.toFixed(0) : n.toFixed(1)) + "%"
}

// htop's TIME+ column: m:ss.cc below an hour, then h:mm:ss, then days.
function cpuTime(sec) {
    sec = Number(sec) || 0
    const pad = (v) => (v < 10 ? "0" : "") + v
    if (sec < 3600) {
        const m = Math.floor(sec / 60)
        const s = sec - m * 60
        const cs = Math.floor((s - Math.floor(s)) * 100)
        return m + ":" + pad(Math.floor(s)) + "." + pad(cs)
    }
    if (sec < 86400 * 10) {
        const h = Math.floor(sec / 3600)
        return h + "h" + pad(Math.floor(sec % 3600 / 60)) + ":" + pad(Math.floor(sec % 60))
    }
    return Math.floor(sec / 86400) + "d" + pad(Math.floor(sec % 86400 / 3600)) + "h"
}

function uptime(sec) {
    sec = Math.floor(Number(sec) || 0)
    const d = Math.floor(sec / 86400), h = Math.floor(sec % 86400 / 3600), m = Math.floor(sec % 3600 / 60)
    return (d > 0 ? d + (d === 1 ? " day, " : " days, ") : "") + (h < 10 ? "0" : "") + h + ":" + (m < 10 ? "0" : "") + m
}

function escape(s) {
    return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
}

// Stable colour per container name.
function hue(s) {
    let h = 0
    for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0
    return (h % 360) / 360
}
