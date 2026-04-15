import QtQuick
import Quickshell.Io
import "."

Item {
    id: ipcRoot

    // Power Menu Handler
    IpcHandler {
        target: "powermenu"
        function toggle(): void {
            PowerState.showPowerMenu = !PowerState.showPowerMenu;
        }
    }

    // Theme Switcher Handler
    IpcHandler {
        target: "theme"
        function open(): void {
            GlobalState.showThemeSwitcher = true;
        }
        function close(): void {
            GlobalState.showThemeSwitcher = false;
        }
        function toggle(): void {
            GlobalState.showThemeSwitcher = !GlobalState.showThemeSwitcher;
        }
        function set(themeName: string): void {
            Theme.switchTheme(themeName);
        }
    }

    // Lockscreen Handler
    IpcHandler {
        target: "lockscreen"
        function lock(): void {
            GlobalState.lockRequested();
        }
    }

    // Screenshot Handler
    IpcHandler {
        target: "screenshot"
        function start(): void {
            GlobalState.screenshotOverlayOpen = true;
        }
    }

    // Dashboard Handler
    IpcHandler {
        target: "dashboard"
        function open(): void {
            DashboardState.showMenu = true;
        }
        function close(): void {
            DashboardState.showMenu = false;
        }
        function toggle(): void {
            DashboardState.showMenu = !DashboardState.showMenu;
        }
    }

    // Notifications Handler
    IpcHandler {
        target: "notifications"
        function open(): void {
            NotificationState.showCenter = true;
        }
        function close(): void {
            NotificationState.showCenter = false;
        }
        function toggle(): void {
            NotificationState.showCenter = !NotificationState.showCenter;
        }
        function clear(): void {
            NotificationState.clearAllNotifications();
        }
        function toggleDnd(): void {
            NotificationState.setDoNotDisturb(!NotificationState.doNotDisturb);
        }
    }

    // Daemon Handler (Routing snapshots to states)
    IpcHandler {
        target: "daemon"

        function audio(payloadText: string): void {
            GlobalState.applyDaemonAudioSnapshot(payloadText);
        }

        function wifi(payloadText: string): void {
            GlobalState.applyDaemonWifiSnapshot(payloadText);
        }

        function bluetooth(payloadText: string): void {
            GlobalState.applyDaemonBluetoothSnapshot(payloadText);
        }

        function music(payloadText: string): void {
            MusicProvider.applyDaemonMusicSnapshot(payloadText);
        }

        function dashboard(payloadText: string): void {
            DashboardState.applyDaemonSnapshot(payloadText);
        }

        function polkit(payloadText: string): void {
            console.log("[IPC] Received Polkit update: " + payloadText);
            const payload = GlobalState.parseDaemonPayload(payloadText);
            if (payload)
                PolkitState.applyDaemonSnapshot(payload);
        }
    }
}
