pragma Singleton
import QtQuick
import Quickshell.Io
import "DaemonRpc.js" as DaemonRpc

QtObject {
    id: polkitState

    property bool active: false
    property string action: ""
    property string message: ""
    property string icon: ""
    property string cookie: ""
    property string user: ""
    property bool verifying: false

    signal authFinished(bool success)

    property Process rpcProcess: Process {
        stdout: StdioCollector {
            onStreamFinished: {
                const rawText = this.text.trim();
                if (!rawText) {
                    polkitState.verifying = false;
                    return;
                }
                try {
                    const start = rawText.indexOf("{");
                    const end = rawText.lastIndexOf("}");
                    if (start === -1 || end === -1) throw new Error("No JSON object found");
                    const jsonText = rawText.substring(start, end + 1);
                    
                    const response = JSON.parse(jsonText);
                    const result = response.result || {};
                    
                    if (result.success) {
                        polkitState.authFinished(true);
                        polkitState.active = false;
                        polkitState.verifying = false;
                        return;
                    }
                    
                    if (result.retry) {
                        // Soft failure: allow user to try again
                        // The daemon broadcast will update the message
                        polkitState.verifying = false;
                        polkitState.authFinished(false);
                        return;
                    }
                } catch (e) {
                    console.error("Polkit: Failed to parse response", e, "Raw text:", rawText);
                }
                
                // Hard failure or error: finalize
                polkitState.verifying = false;
                polkitState.authFinished(false);
            }
        }
    }

    function applyDaemonSnapshot(payload) {
        if (payload.polkit) {
            const data = payload.polkit;
            
            // Update message first so it's available for onActiveChanged/onAuthFinished
            if (data.message !== undefined) polkitState.message = data.message;
            
            if (data.active !== undefined) {
                const wasActive = polkitState.active;
                polkitState.active = data.active;
                
                // If the daemon just told us we're no longer active, 
                // it's likely authentication finished (success or fail).
                if (wasActive && !data.active) {
                    polkitState.authFinished(data.success === true);
                    polkitState.verifying = false;
                }
            }
            
            if (data.active) {
                if (data.action !== undefined) polkitState.action = data.action;
                if (data.icon !== undefined) polkitState.icon = data.icon;
                if (data.cookie !== undefined) polkitState.cookie = data.cookie;
                if (data.user !== undefined) polkitState.user = data.user;
            }
        }
    }

    function respond(password) {
        if (!polkitState.cookie) return;
        polkitState.verifying = true;
        
        rpcProcess.command = DaemonRpc.command("polkit.respond", {
            user: polkitState.user,
            password: password,
            cookie: polkitState.cookie
        });
        rpcProcess.running = true;
    }

    function cancel() {
        active = false;
    }
}
