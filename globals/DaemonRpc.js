.pragma library

var consecutiveFailures = 0;
var cooldownUntilMs = 0;

function nowMs() {
    return Date.now();
}

function canUse() {
    return nowMs() >= cooldownUntilMs;
}

function recordSuccess() {
    consecutiveFailures = 0;
    cooldownUntilMs = 0;
}

function recordFailure() {
    consecutiveFailures = Math.min(7, consecutiveFailures + 1);
    const backoffMs = Math.min(15000, 400 * Math.pow(2, Math.max(0, consecutiveFailures - 1)));
    cooldownUntilMs = nowMs() + backoffMs;
}

function shellSingleQuote(value) {
    return "'" + String(value || "").replace(/'/g, "'\"'\"'") + "'";
}

function command(method, params, timeoutSec) {
    const request = JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: String(method || ""),
        params: params || {}
    });
    const waitSec = Math.max(1, Math.round(Number(timeoutSec) || 1));
    const cmd = "printf '%s\\n' " + shellSingleQuote(request) + " | nc -U -w " + String(waitSec) + " \"$XDG_RUNTIME_DIR/stratumd.sock\"";
    return ["sh", "-lc", cmd];
}