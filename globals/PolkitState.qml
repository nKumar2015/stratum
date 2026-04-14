pragma Singleton
import QtQuick
import Quickshell.Io

QtObject {
    id: polkitState

    property bool active: false
    property string action: ""
    property string message: ""
    property string icon: ""
    property string cookie: ""
    property string user: "root"

    signal authFinished(bool success)

    function applyDaemonSnapshot(payload) {
        if (payload.polkit) {
            const data = payload.polkit;
            if (data.active !== undefined) active = data.active;
            if (data.active) {
                if (data.action !== undefined) action = data.action;
                if (data.message !== undefined) message = data.message;
                if (data.icon !== undefined) icon = data.icon;
                if (data.cookie !== undefined) cookie = data.cookie;
            }
        }
    }

    function respond(password) {
        if (!cookie) return;
        
        const payload = JSON.stringify({
            jsonrpc: "2.0",
            id: 1,
            method: "polkit.respond",
            params: {
                user: user,
                password: password,
                cookie: cookie
            }
        });

        const rpc = Process.run(["sh", "-c", `printf '%s\\n' '${payload}' | nc -U -w 2 "$XDG_RUNTIME_DIR/stratumd.sock"`]);
        
        rpc.finished.connect((exitCode) => {
            if (exitCode === 0) {
                try {
                    const out = rpc.stdout.readAll();
                    const response = JSON.parse(out);
                    if (response.result && response.result.success) {
                        authFinished(true);
                        active = false;
                        return;
                    }
                } catch (e) {
                    console.error("Polkit: Failed to parse response", e);
                }
            }
            authFinished(false);
        });
    }

    function cancel() {
        active = false;
        // In a real implementation, we would notify the daemon to cancel too
    }
}
