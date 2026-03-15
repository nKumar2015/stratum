import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import Quickshell.Io

import "../theme"
import "../globals"

PanelWindow {
    id: dashboard

    property bool loading: false
    property string lastError: ""

    property string calendarTitle: ""
    property var calendarWeekdays: ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"]
    property var calendarRows: [
        [0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0]
    ]
    property int todayDay: -1
    property int selectedCalendarYear: (new Date()).getFullYear()
    property int selectedCalendarMonth: (new Date()).getMonth() + 1
    property int calendarYear: selectedCalendarYear
    property int calendarMonth: selectedCalendarMonth
    property int calendarDaysInMonth: 30
    property int calendarFirstWeekday: 0
    property int calendarCellWidth: 52
    property int calendarCellHeight: 36
    property int calendarGridGap: 3
    property int calendarWeekdayHeight: 30
    property int calendarWeekdayGap: 3
    property int calendarWeekdayToDatesGap: 2
    readonly property int calendarGridWidth: (calendarCellWidth * 7) + (calendarGridGap * 6)
    property real calendarSwipeX: 0
    property var calendarCache: ({})
    property var calendarPrefetchQueue: []
    property bool calendarPrefetchRunning: false
    property int calendarPrefetchYear: 0
    property int calendarPrefetchMonth: 0

    property string musicStatus: "Unknown"
    property string musicPlayer: "N/A"
    property string musicTitle: "Nothing playing"
    property string musicArtist: "N/A"
    property string musicAlbum: "N/A"
    property string musicPlayerTitle: "N/A"
    property string musicPosition: "00:00"
    property string musicLength: "00:00"
    property string musicArtUrl: ""

    property int cpuPercent: 0
    property string gpuPercentText: "N/A"
    property int gpuPercentValue: 0
    property string gpuSource: "N/A"
    property real ramUsedGiB: 0
    property real ramTotalGiB: 0
    property int ramPercent: 0
    property real storageUsedGiB: 0
    property real storageTotalGiB: 0
    property int storagePercent: 0

    function parseNumber(value, fallback) {
        const n = Number(value);
        return isNaN(n) ? fallback : n;
    }

    function clampPercent(value) {
        const n = parseInt(value);
        if (isNaN(n))
            return 0;
        return Math.max(0, Math.min(100, n));
    }

    function metricColor(percent) {
        if (percent >= 85)
            return Theme.red;
        if (percent >= 65)
            return Theme.yellow;
        return Theme.green;
    }

    function resetCalendarRows() {
        calendarRows = [
            [0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0]
        ];
    }

    function createEmptyCalendarRows() {
        return [
            [0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0]
        ];
    }

    function calendarCacheKey(year, month) {
        return String(year) + "-" + String(month);
    }

    function parseCalendarPayload(rawText) {
        const payload = {
            hasCalendar: false,
            title: calendarTitle,
            weekdays: calendarWeekdays.slice(),
            rows: createEmptyCalendarRows(),
            today: todayDay,
            year: selectedCalendarYear,
            month: selectedCalendarMonth,
            daysInMonth: calendarDaysInMonth,
            firstWeekday: calendarFirstWeekday
        };

        const lines = String(rawText || "").trim().split("\n");
        let rowIndex = 0;
        for (let i = 0; i < lines.length; i++) {
            const line = (lines[i] || "").trim();
            if (!line || line.startsWith("__ERROR__|"))
                continue;

            const parts = line.split("|");
            const type = (parts[0] || "").trim();

            if (type === "CAL_TITLE") {
                payload.title = (parts[1] || "").trim();
                payload.hasCalendar = true;
            } else if (type === "CAL_META") {
                const y = parseInt(parts[1] || String(payload.year));
                const m = parseInt(parts[2] || String(payload.month));
                const dim = parseInt(parts[3] || String(payload.daysInMonth));
                const fwd = parseInt(parts[4] || String(payload.firstWeekday));

                payload.year = isNaN(y) ? payload.year : y;
                payload.month = isNaN(m) ? payload.month : m;
                payload.daysInMonth = isNaN(dim) ? payload.daysInMonth : dim;
                payload.firstWeekday = isNaN(fwd) ? payload.firstWeekday : Math.max(0, Math.min(6, fwd));
                payload.hasCalendar = true;
            } else if (type === "CAL_WEEKDAYS") {
                payload.weekdays = [
                    (parts[1] || "Su").trim(),
                    (parts[2] || "Mo").trim(),
                    (parts[3] || "Tu").trim(),
                    (parts[4] || "We").trim(),
                    (parts[5] || "Th").trim(),
                    (parts[6] || "Fr").trim(),
                    (parts[7] || "Sa").trim()
                ];
                payload.hasCalendar = true;
            } else if (type === "CAL_ROW") {
                if (rowIndex < 6) {
                    const row = [];
                    for (let c = 1; c <= 7; c++) {
                        const day = parseInt(parts[c] || "0");
                        row.push(isNaN(day) ? 0 : day);
                    }
                    payload.rows[rowIndex] = row;
                    rowIndex++;
                    payload.hasCalendar = true;
                }
            } else if (type === "TODAY") {
                const day = parseInt(parts[1] || "-1");
                payload.today = isNaN(day) ? -1 : day;
                payload.hasCalendar = true;
            }
        }

        return payload;
    }

    function applyCalendarPayload(payload, cacheIt) {
        if (!payload || !payload.hasCalendar)
            return;

        calendarTitle = payload.title;
        calendarWeekdays = payload.weekdays.slice();
        calendarRows = payload.rows.map(row => row.slice());
        todayDay = payload.today;
        calendarYear = payload.year;
        calendarMonth = payload.month;
        selectedCalendarYear = payload.year;
        selectedCalendarMonth = payload.month;
        calendarDaysInMonth = payload.daysInMonth;
        calendarFirstWeekday = payload.firstWeekday;

        if (cacheIt) {
            cacheCalendarPayload(payload);
        }
    }

    function cacheCalendarPayload(payload) {
        if (!payload || !payload.hasCalendar)
            return;

        const key = calendarCacheKey(payload.year, payload.month);
        const next = {};
        for (const existingKey in calendarCache)
            next[existingKey] = calendarCache[existingKey];

        next[key] = {
            title: payload.title,
            weekdays: payload.weekdays.slice(),
            rows: payload.rows.map(row => row.slice()),
            today: payload.today,
            year: payload.year,
            month: payload.month,
            daysInMonth: payload.daysInMonth,
            firstWeekday: payload.firstWeekday
        };
        calendarCache = next;
    }

    function applyCachedCalendar(year, month) {
        const key = calendarCacheKey(year, month);
        const cached = calendarCache[key];
        if (!cached)
            return false;

        applyCalendarPayload({
            hasCalendar: true,
            title: cached.title,
            weekdays: cached.weekdays,
            rows: cached.rows,
            today: cached.today,
            year: cached.year,
            month: cached.month,
            daysInMonth: cached.daysInMonth,
            firstWeekday: cached.firstWeekday
        }, false);
        return true;
    }

    function shiftedYearMonth(year, month, offset) {
        const ref = new Date(year, month - 1, 1);
        ref.setMonth(ref.getMonth() + offset);
        return {
            year: ref.getFullYear(),
            month: ref.getMonth() + 1
        };
    }

    function runCalendarPrefetch() {
        if (calendarPrefetchRunning)
            return;
        if (!calendarPrefetchQueue || calendarPrefetchQueue.length === 0)
            return;

        const next = calendarPrefetchQueue[0];
        calendarPrefetchYear = next.year;
        calendarPrefetchMonth = next.month;
        calendarPrefetchRunning = true;
        calendarPrefetchProc.command = [
            "sh",
            Quickshell.shellDir + "/scripts/dashboard_menu.sh",
            "calendar",
            String(calendarPrefetchYear),
            String(calendarPrefetchMonth)
        ];
        calendarPrefetchProc.running = true;
    }

    function queueCalendarPrefetch(year, month) {
        if (year < 1 || month < 1 || month > 12)
            return;

        const key = calendarCacheKey(year, month);
        if (calendarCache[key])
            return;
        if (year === selectedCalendarYear && month === selectedCalendarMonth)
            return;
        if (calendarPrefetchRunning && calendarPrefetchYear === year && calendarPrefetchMonth === month)
            return;

        for (let i = 0; i < calendarPrefetchQueue.length; i++) {
            const pending = calendarPrefetchQueue[i];
            if (pending.year === year && pending.month === month)
                return;
        }

        calendarPrefetchQueue = calendarPrefetchQueue.concat([{ year: year, month: month }]);
        runCalendarPrefetch();
    }

    function preloadNearbyCalendars(year, month) {
        const offsets = [-2, -1, 1, 2];
        for (let i = 0; i < offsets.length; i++) {
            const shifted = shiftedYearMonth(year, month, offsets[i]);
            queueCalendarPrefetch(shifted.year, shifted.month);
        }
    }

    function daysInMonth(year, month) {
        const y = Number(year);
        const m = Number(month);
        if (isNaN(y) || isNaN(m) || m < 1 || m > 12)
            return 30;
        return new Date(y, m, 0).getDate();
    }

    function changeCalendarMonth(offset) {
        const current = new Date(selectedCalendarYear, selectedCalendarMonth - 1, 1);
        current.setMonth(current.getMonth() + offset);
        selectedCalendarYear = current.getFullYear();
        selectedCalendarMonth = current.getMonth() + 1;
        applyCachedCalendar(selectedCalendarYear, selectedCalendarMonth);
        preloadNearbyCalendars(selectedCalendarYear, selectedCalendarMonth);
        refreshDashboard();
    }

    function changeCalendarYear(offset) {
        let y = selectedCalendarYear + offset;
        if (y < 1)
            y = 1;
        selectedCalendarYear = y;
        applyCachedCalendar(selectedCalendarYear, selectedCalendarMonth);
        preloadNearbyCalendars(selectedCalendarYear, selectedCalendarMonth);
        refreshDashboard();
    }

    function jumpCalendarToToday() {
        const now = new Date();
        selectedCalendarYear = now.getFullYear();
        selectedCalendarMonth = now.getMonth() + 1;
        applyCachedCalendar(selectedCalendarYear, selectedCalendarMonth);
        preloadNearbyCalendars(selectedCalendarYear, selectedCalendarMonth);
        refreshDashboard();
    }

    function resetCalendarSelection() {
        const now = new Date();
        selectedCalendarYear = now.getFullYear();
        selectedCalendarMonth = now.getMonth() + 1;
        calendarYear = selectedCalendarYear;
        calendarMonth = selectedCalendarMonth;
        todayDay = now.getDate();
    }

    function gridDayValue(index) {
        const row = Math.floor(index / 7);
        const col = index % 7;
        const rowData = calendarRows[row] || [0, 0, 0, 0, 0, 0, 0];
        const day = parseInt(rowData[col] || 0);

        if (!isNaN(day) && day > 0)
            return day;

        const linear = row * 7 + col;
        if (linear < calendarFirstWeekday) {
            const prevMonth = selectedCalendarMonth === 1 ? 12 : selectedCalendarMonth - 1;
            const prevYear = selectedCalendarMonth === 1 ? selectedCalendarYear - 1 : selectedCalendarYear;
            const prevDays = daysInMonth(prevYear, prevMonth);
            return prevDays - calendarFirstWeekday + linear + 1;
        }

        return linear - calendarFirstWeekday - calendarDaysInMonth + 1;
    }

    function gridCellCurrentMonth(index) {
        const row = Math.floor(index / 7);
        const col = index % 7;
        const rowData = calendarRows[row] || [0, 0, 0, 0, 0, 0, 0];
        const day = parseInt(rowData[col] || 0);
        return !isNaN(day) && day > 0;
    }

    function gridCellIsToday(index) {
        if (todayDay <= 0)
            return false;
        if (!gridCellCurrentMonth(index))
            return false;
        return gridDayValue(index) === todayDay;
    }

    function weekdayLabel(shortLabel) {
        const key = String(shortLabel || "").toLowerCase();
        if (key === "su")
            return "Sun";
        if (key === "mo")
            return "Mon";
        if (key === "tu")
            return "Tue";
        if (key === "we")
            return "Wed";
        if (key === "th")
            return "Thu";
        if (key === "fr")
            return "Fri";
        if (key === "sa")
            return "Sat";
        return String(shortLabel || "");
    }

    function refreshDashboard() {
        loading = true;
        lastError = "";
        dataProc.command = [
            "sh",
            Quickshell.shellDir + "/scripts/dashboard_menu.sh",
            "all",
            String(selectedCalendarYear),
            String(selectedCalendarMonth)
        ];
        dataProc.running = true;
    }

    function animateCalendarSwipe(direction) {
        const dir = direction < 0 ? -1 : 1;
        calendarSwipeX = 42 * dir;
        calendarDatesAnimatedLayer.opacity = 0.75;
        calendarSwipeAnimation.restart();
    }

    function sendPlayerAction(action) {
        if (!action || action.length === 0)
            return;

        if (!musicPlayer || musicPlayer === "N/A" || musicPlayer === "None") {
            lastError = "No active player";
            return;
        }

        controlProc.command = ["playerctl", "-p", musicPlayer, action];
        controlProc.running = true;
    }

    anchors {
        top: true
        bottom: true
        left: true
        right: true
    }

    color: "#90000000"
    exclusiveZone: -1
    visible: GlobalState.showDashboardMenu

    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: visible ? WlrKeyboardFocus.OnDemand : WlrKeyboardFocus.None

    onVisibleChanged: {
        if (visible) {
            applyCachedCalendar(selectedCalendarYear, selectedCalendarMonth);
            preloadNearbyCalendars(selectedCalendarYear, selectedCalendarMonth);
            refreshDashboard();
            refreshTimer.restart();
        } else {
            refreshTimer.stop();
            resetCalendarSelection();
        }
    }

    IpcHandler {
        target: "dashboard"

        function open(): void {
            GlobalState.showDashboardMenu = true;
        }

        function close(): void {
            GlobalState.showDashboardMenu = false;
        }

        function toggle(): void {
            GlobalState.showDashboardMenu = !GlobalState.showDashboardMenu;
        }
    }

    Shortcut {
        sequence: "Escape"
        onActivated: {
            if (GlobalState.showDashboardMenu)
                GlobalState.showDashboardMenu = false;
        }
    }

    Timer {
        id: refreshTimer
        interval: 2000
        repeat: true
        running: false
        onTriggered: {
            if (GlobalState.showDashboardMenu)
                dashboard.refreshDashboard();
        }
    }

    Process {
        id: dataProc
        command: ["sh", Quickshell.shellDir + "/scripts/dashboard_menu.sh", "all", String(selectedCalendarYear), String(selectedCalendarMonth)]
        running: false
        stdout: StdioCollector {
            onStreamFinished: {
                dashboard.loading = false;

                const raw = this.text.trim();
                if (!raw) {
                    dashboard.lastError = "No dashboard data";
                    return;
                }

                const calendarPayload = dashboard.parseCalendarPayload(raw);
                if (calendarPayload.hasCalendar) {
                    dashboard.cacheCalendarPayload(calendarPayload);

                    // Avoid visual jump: only apply responses for the currently selected month.
                    if (calendarPayload.year === dashboard.selectedCalendarYear && calendarPayload.month === dashboard.selectedCalendarMonth) {
                        dashboard.applyCalendarPayload(calendarPayload, false);
                        dashboard.preloadNearbyCalendars(calendarPayload.year, calendarPayload.month);
                    }
                }

                const lines = raw.split("\n");
                let parseError = "";

                for (let i = 0; i < lines.length; i++) {
                    const line = lines[i].trim();
                    if (!line)
                        continue;

                    if (line.startsWith("__ERROR__|")) {
                        parseError = line.replace("__ERROR__|", "");
                        continue;
                    }

                    const parts = line.split("|");
                    const type = (parts[0] || "").trim();

                    if (type === "MUSIC") {
                        dashboard.musicStatus = (parts[1] || "Unknown").trim();
                        dashboard.musicPlayer = (parts[2] || "N/A").trim();
                        dashboard.musicTitle = (parts[3] || "Nothing playing").trim();
                        dashboard.musicArtist = (parts[4] || "N/A").trim();
                        dashboard.musicAlbum = (parts[5] || "N/A").trim();
                        dashboard.musicPosition = (parts[6] || "00:00").trim();
                        dashboard.musicLength = (parts[7] || "00:00").trim();
                        dashboard.musicArtUrl = (parts[8] || "").trim();
                        dashboard.musicPlayerTitle = (parts[9] || dashboard.musicPlayer || "N/A").trim();
                    } else if (type === "CPU") {
                        dashboard.cpuPercent = dashboard.clampPercent(parts[1] || "0");
                    } else if (type === "GPU") {
                        dashboard.gpuPercentText = (parts[1] || "N/A").trim();
                        const gpuVal = parseInt(dashboard.gpuPercentText);
                        dashboard.gpuPercentValue = isNaN(gpuVal) ? 0 : dashboard.clampPercent(String(gpuVal));
                        dashboard.gpuSource = (parts[2] || "N/A").trim();
                    } else if (type === "RAM") {
                        dashboard.ramUsedGiB = dashboard.parseNumber(parts[1] || "0", 0);
                        dashboard.ramTotalGiB = dashboard.parseNumber(parts[2] || "0", 0);
                        dashboard.ramPercent = dashboard.clampPercent(parts[3] || "0");
                    } else if (type === "STORAGE") {
                        dashboard.storageUsedGiB = dashboard.parseNumber(parts[1] || "0", 0);
                        dashboard.storageTotalGiB = dashboard.parseNumber(parts[2] || "0", 0);
                        dashboard.storagePercent = dashboard.clampPercent(parts[3] || "0");
                    }
                }

                dashboard.lastError = parseError;
            }
        }
    }

    Process {
        id: calendarPrefetchProc
        running: false
        stdout: StdioCollector {
            onStreamFinished: {
                const raw = this.text.trim();
                const payload = dashboard.parseCalendarPayload(raw);
                if (payload.hasCalendar)
                    dashboard.cacheCalendarPayload(payload);

                if (dashboard.calendarPrefetchQueue.length > 0)
                    dashboard.calendarPrefetchQueue = dashboard.calendarPrefetchQueue.slice(1);

                dashboard.calendarPrefetchRunning = false;
                dashboard.runCalendarPrefetch();
            }
        }
    }

    Process {
        id: controlProc
        running: false
        stdout: StdioCollector {
            onStreamFinished: {
                dashboard.refreshDashboard();
            }
        }
    }

    MouseArea {
        anchors.fill: parent
        onClicked: GlobalState.showDashboardMenu = false
    }

    Rectangle {
        id: panel
        width: Math.min(parent.width - 20, panelContentRow.implicitWidth + 16)
        height: panelContentRow.implicitHeight + (dashboard.lastError.length > 0 ? 18 : 0) + 16
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: parent.top
        anchors.topMargin: GlobalState.showDashboardMenu ? 12 : -height - 18

        color: Theme.background
        radius: 14
        border.width: 1
        border.color: Theme.grey

        Behavior on anchors.topMargin {
            NumberAnimation {
                duration: 220
                easing.type: Easing.OutCubic
            }
        }

        MouseArea {
            anchors.fill: parent
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 8
            spacing: 4

            RowLayout {
                visible: dashboard.lastError.length > 0
                Layout.fillWidth: true
                Layout.preferredHeight: visible ? implicitHeight : 0
                Layout.minimumHeight: visible ? implicitHeight : 0
                Item { Layout.fillWidth: true }

                Text {
                    text: dashboard.lastError
                    color: Theme.red
                    font.family: Theme.font
                    font.pixelSize: 10
                    elide: Text.ElideRight
                    Layout.maximumWidth: 280
                }
            }

            RowLayout {
                id: panelContentRow
                Layout.fillWidth: true
                Layout.fillHeight: false
                spacing: 8

                Rectangle {
                    id: performancePanel
                    Layout.alignment: Qt.AlignTop
                    Layout.preferredWidth: 270
                    Layout.preferredHeight: Math.max(performanceColumn.implicitHeight + 20, musicColumn.implicitHeight + 20)
                    Layout.minimumHeight: Math.max(performanceColumn.implicitHeight + 20, musicColumn.implicitHeight + 20)
                    Layout.maximumHeight: Math.max(performanceColumn.implicitHeight + 20, musicColumn.implicitHeight + 20)
                    color: Theme.background
                    radius: 10
                    border.width: 1
                    border.color: Theme.grey

                    ColumnLayout {
                        id: performanceColumn
                        anchors.fill: parent
                        anchors.margins: 10
                        spacing: 6

                        Text {
                            text: "Performance"
                            color: Theme.text
                            font.family: Theme.font
                            font.pixelSize: 14
                            font.bold: true
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            implicitHeight: 70
                            radius: 8
                            color: Theme.background
                            border.width: 1
                            border.color: Theme.grey

                            ColumnLayout {
                                anchors.fill: parent
                                anchors.margins: 8
                                spacing: 2

                                RowLayout {
                                    spacing: 4

                                    Text {
                                        text: ""
                                        color: Theme.activeWs
                                        font.family: Theme.font
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        text: "CPU"
                                        color: Theme.inactiveWs
                                        font.family: Theme.font
                                        font.pixelSize: 10
                                        font.bold: true
                                    }
                                }

                                Text {
                                    text: String(dashboard.cpuPercent) + "%"
                                    color: dashboard.metricColor(dashboard.cpuPercent)
                                    font.family: Theme.font
                                    font.pixelSize: 17
                                    font.bold: true
                                }

                                Rectangle {
                                    Layout.fillWidth: true
                                    implicitHeight: 6
                                    radius: 3
                                    color: Theme.black

                                    Rectangle {
                                        width: parent.width * dashboard.cpuPercent / 100
                                        height: parent.height
                                        radius: 3
                                        color: dashboard.metricColor(dashboard.cpuPercent)

                                        Behavior on width {
                                            NumberAnimation {
                                                duration: 260
                                                easing.type: Easing.OutCubic
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            implicitHeight: 70
                            radius: 8
                            color: Theme.background
                            border.width: 1
                            border.color: Theme.grey

                            ColumnLayout {
                                anchors.fill: parent
                                anchors.margins: 8
                                spacing: 2

                                RowLayout {
                                    spacing: 4

                                    Text {
                                        text: "󰢮"
                                        color: Theme.activeWs
                                        font.family: Theme.font
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        text: "GPU"
                                        color: Theme.inactiveWs
                                        font.family: Theme.font
                                        font.pixelSize: 10
                                        font.bold: true
                                    }
                                }

                                Text {
                                    text: dashboard.gpuPercentText === "N/A" ? "N/A" : dashboard.gpuPercentText + "%"
                                    color: dashboard.gpuPercentText === "N/A" ? Theme.inactiveWs : Theme.green
                                    font.family: Theme.font
                                    font.pixelSize: 17
                                    font.bold: true
                                }

                                Text {
                                    text: dashboard.gpuSource
                                    color: Theme.inactiveWs
                                    font.family: Theme.font
                                    font.pixelSize: 9
                                }

                                Rectangle {
                                    Layout.fillWidth: true
                                    implicitHeight: 6
                                    radius: 3
                                    color: Theme.black
                                    visible: dashboard.gpuPercentText !== "N/A"

                                    Rectangle {
                                        width: parent.width * dashboard.gpuPercentValue / 100
                                        height: parent.height
                                        radius: 3
                                        color: Theme.green

                                        Behavior on width {
                                            NumberAnimation {
                                                duration: 260
                                                easing.type: Easing.OutCubic
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            implicitHeight: 82
                            radius: 8
                            color: Theme.background
                            border.width: 1
                            border.color: Theme.grey

                            ColumnLayout {
                                anchors.fill: parent
                                anchors.margins: 8
                                spacing: 2

                                RowLayout {
                                    spacing: 4

                                    Text {
                                        text: "󰍛"
                                        color: Theme.activeWs
                                        font.family: Theme.font
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        text: "RAM"
                                        color: Theme.inactiveWs
                                        font.family: Theme.font
                                        font.pixelSize: 10
                                        font.bold: true
                                    }
                                }

                                Text {
                                    text: dashboard.ramUsedGiB.toFixed(1) + " / " + dashboard.ramTotalGiB.toFixed(1) + " GiB"
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 12
                                }

                                Text {
                                    text: String(dashboard.ramPercent) + "%"
                                    color: dashboard.metricColor(dashboard.ramPercent)
                                    font.family: Theme.font
                                    font.pixelSize: 13
                                    font.bold: true
                                }

                                Rectangle {
                                    Layout.fillWidth: true
                                    implicitHeight: 6
                                    radius: 3
                                    color: Theme.black

                                    Rectangle {
                                        width: parent.width * dashboard.ramPercent / 100
                                        height: parent.height
                                        radius: 3
                                        color: dashboard.metricColor(dashboard.ramPercent)

                                        Behavior on width {
                                            NumberAnimation {
                                                duration: 260
                                                easing.type: Easing.OutCubic
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            implicitHeight: 82
                            radius: 8
                            color: Theme.background
                            border.width: 1
                            border.color: Theme.grey

                            ColumnLayout {
                                anchors.fill: parent
                                anchors.margins: 8
                                spacing: 2

                                RowLayout {
                                    spacing: 4

                                    Text {
                                        text: ""
                                        color: Theme.activeWs
                                        font.family: Theme.font
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        text: "Storage"
                                        color: Theme.inactiveWs
                                        font.family: Theme.font
                                        font.pixelSize: 10
                                        font.bold: true
                                    }
                                }

                                Text {
                                    text: dashboard.storageUsedGiB.toFixed(1) + " / " + dashboard.storageTotalGiB.toFixed(1) + " GiB"
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 12
                                }

                                Text {
                                    text: String(dashboard.storagePercent) + "%"
                                    color: dashboard.metricColor(dashboard.storagePercent)
                                    font.family: Theme.font
                                    font.pixelSize: 13
                                    font.bold: true
                                }

                                Rectangle {
                                    Layout.fillWidth: true
                                    implicitHeight: 6
                                    radius: 3
                                    color: Theme.black

                                    Rectangle {
                                        width: parent.width * dashboard.storagePercent / 100
                                        height: parent.height
                                        radius: 3
                                        color: dashboard.metricColor(dashboard.storagePercent)

                                        Behavior on width {
                                            NumberAnimation {
                                                duration: 260
                                                easing.type: Easing.OutCubic
                                            }
                                        }
                                    }
                                }
                            }
                        }

                    }
                }

                Rectangle {
                    id: calendarPanel
                    Layout.alignment: Qt.AlignTop
                    Layout.preferredWidth: dashboard.calendarGridWidth + 8
                    Layout.minimumWidth: dashboard.calendarGridWidth + 8
                    Layout.maximumWidth: dashboard.calendarGridWidth + 8
                    Layout.preferredHeight: Math.max(performanceColumn.implicitHeight + 20, musicColumn.implicitHeight + 20)
                    Layout.minimumHeight: Math.max(performanceColumn.implicitHeight + 20, musicColumn.implicitHeight + 20)
                    Layout.maximumHeight: Math.max(performanceColumn.implicitHeight + 20, musicColumn.implicitHeight + 20)
                    color: Theme.background
                    radius: 10
                    border.width: 1
                    border.color: Theme.grey

                    ColumnLayout {
                        id: calendarColumn
                        anchors.fill: parent
                        anchors.margins: 4
                        spacing: 0

                        Text {
                            text: dashboard.calendarTitle.length > 0 ? dashboard.calendarTitle : "Calendar"
                            color: Theme.text
                            font.family: Theme.font
                            font.pixelSize: 16
                            font.bold: true
                            horizontalAlignment: Text.AlignHCenter
                            Layout.fillWidth: true
                            Layout.topMargin: 2
                            Layout.bottomMargin: 8
                        }

                        Item {
                            Layout.alignment: Qt.AlignHCenter
                            Layout.preferredWidth: dashboard.calendarGridWidth
                            Layout.minimumWidth: dashboard.calendarGridWidth
                            Layout.maximumWidth: dashboard.calendarGridWidth
                            implicitHeight: dashboard.calendarWeekdayHeight + dashboard.calendarWeekdayToDatesGap + (dashboard.calendarCellHeight * 6) + (dashboard.calendarGridGap * 5)

                            GridLayout {
                                anchors.top: parent.top
                                anchors.horizontalCenter: parent.horizontalCenter
                                columns: 7
                                rowSpacing: 0
                                columnSpacing: dashboard.calendarWeekdayGap
                                width: dashboard.calendarGridWidth

                                Repeater {
                                    model: dashboard.calendarWeekdays
                                    delegate: Rectangle {
                                        required property var modelData
                                        implicitWidth: dashboard.calendarCellWidth
                                        implicitHeight: dashboard.calendarWeekdayHeight
                                        radius: 6
                                        color: Theme.background
                                        border.width: 1

                                        Text {
                                            anchors.fill: parent
                                            anchors.margins: 3
                                            text: dashboard.weekdayLabel(modelData)
                                            color: Theme.defaultWs
                                            font.family: Theme.font
                                            font.pixelSize: 11
                                            font.bold: true
                                            horizontalAlignment: Text.AlignHCenter
                                            verticalAlignment: Text.AlignVCenter
                                        }
                                    }
                                }
                            }

                            Item {
                                id: calendarDatesAnimatedLayer
                                anchors.top: parent.top
                                anchors.topMargin: dashboard.calendarWeekdayHeight + dashboard.calendarWeekdayToDatesGap
                                anchors.horizontalCenter: parent.horizontalCenter
                                width: dashboard.calendarGridWidth
                                height: (dashboard.calendarCellHeight * 6) + (dashboard.calendarGridGap * 5)
                                transform: Translate {
                                    x: dashboard.calendarSwipeX
                                }

                                Behavior on opacity {
                                    NumberAnimation {
                                        duration: 120
                                        easing.type: Easing.OutCubic
                                    }
                                }

                                GridLayout {
                                    anchors.fill: parent
                                    columns: 7
                                    rowSpacing: dashboard.calendarGridGap
                                    columnSpacing: dashboard.calendarGridGap

                                    Repeater {
                                        model: 42
                                        delegate: Rectangle {
                                            property int dayValue: dashboard.gridDayValue(index)
                                            property bool inCurrentMonth: dashboard.gridCellCurrentMonth(index)
                                            property bool isToday: dashboard.gridCellIsToday(index)

                                            implicitWidth: dashboard.calendarCellWidth
                                            implicitHeight: dashboard.calendarCellHeight
                                            radius: 6
                                            color: isToday ? Theme.activeWs : Theme.background
                                            border.width: isToday ? 1 : 0
                                            border.color: Theme.activeWs

                                            Text {
                                                anchors.centerIn: parent
                                                text: String(dayValue)
                                                color: isToday ? Theme.background : (inCurrentMonth ? Theme.text : Theme.inactiveWs)
                                                font.family: Theme.font
                                                font.pixelSize: 12
                                                font.bold: isToday
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        RowLayout {
                            Layout.alignment: Qt.AlignHCenter
                            Layout.preferredWidth: dashboard.calendarGridWidth
                            Layout.minimumWidth: dashboard.calendarGridWidth
                            Layout.maximumWidth: dashboard.calendarGridWidth
                            Layout.topMargin: 5
                            spacing: 6

                            Rectangle {
                                Layout.preferredWidth: 38
                                Layout.preferredHeight: 28
                                radius: 6
                                color: Theme.background
                                border.width: 1
                                border.color: Theme.grey

                                Text {
                                    anchors.centerIn: parent
                                    text: "<<"
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 11
                                    font.bold: true
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: {
                                        dashboard.changeCalendarYear(-1);
                                        dashboard.animateCalendarSwipe(-1);
                                    }
                                }
                            }

                            Rectangle {
                                Layout.preferredWidth: 38
                                Layout.preferredHeight: 28
                                radius: 6
                                color: Theme.background
                                border.width: 1
                                border.color: Theme.grey

                                Text {
                                    anchors.centerIn: parent
                                    text: "<"
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 12
                                    font.bold: true
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: {
                                        dashboard.changeCalendarMonth(-1);
                                        dashboard.animateCalendarSwipe(-1);
                                    }
                                }
                            }

                            Rectangle {
                                Layout.fillWidth: true
                                Layout.preferredHeight: 28
                                radius: 6
                                color: Theme.background
                                border.width: 1
                                border.color: Theme.grey

                                Text {
                                    anchors.centerIn: parent
                                    text: "Today"
                                    color: Theme.activeWs
                                    font.family: Theme.font
                                    font.pixelSize: 11
                                    font.bold: true
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: {
                                        dashboard.jumpCalendarToToday();
                                        dashboard.animateCalendarSwipe(1);
                                    }
                                }
                            }

                            Rectangle {
                                Layout.preferredWidth: 38
                                Layout.preferredHeight: 28
                                radius: 6
                                color: Theme.background
                                border.width: 1
                                border.color: Theme.grey

                                Text {
                                    anchors.centerIn: parent
                                    text: ">"
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 12
                                    font.bold: true
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: {
                                        dashboard.changeCalendarMonth(1);
                                        dashboard.animateCalendarSwipe(1);
                                    }
                                }
                            }

                            Rectangle {
                                Layout.preferredWidth: 38
                                Layout.preferredHeight: 28
                                radius: 6
                                color: Theme.background
                                border.width: 1
                                border.color: Theme.grey

                                Text {
                                    anchors.centerIn: parent
                                    text: ">>"
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 11
                                    font.bold: true
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: {
                                        dashboard.changeCalendarYear(1);
                                        dashboard.animateCalendarSwipe(1);
                                    }
                                }
                            }
                        }

                    }
                }

                Rectangle {
                    id: musicPanel
                    Layout.alignment: Qt.AlignTop
                    Layout.preferredWidth: 310
                    Layout.preferredHeight: Math.max(performanceColumn.implicitHeight + 20, musicColumn.implicitHeight + 20)
                    Layout.minimumHeight: Math.max(performanceColumn.implicitHeight + 20, musicColumn.implicitHeight + 20)
                    Layout.maximumHeight: Math.max(performanceColumn.implicitHeight + 20, musicColumn.implicitHeight + 20)
                    color: Theme.background
                    radius: 10
                    border.width: 1
                    border.color: Theme.grey

                    ColumnLayout {
                        id: musicColumn
                        anchors.fill: parent
                        anchors.margins: 10
                        spacing: 6

                        Text {
                            text: "Now Playing"
                            color: Theme.text
                            font.family: Theme.font
                            font.pixelSize: 14
                            font.bold: true
                            horizontalAlignment: Text.AlignHCenter
                            Layout.fillWidth: true
                        }

                        Rectangle {
                            Layout.alignment: Qt.AlignHCenter
                            Layout.preferredWidth: 200
                            Layout.preferredHeight: 200
                            radius: 10
                            color: Theme.background
                            border.width: 1
                            border.color: Theme.grey
                            clip: true

                            Image {
                                anchors.fill: parent
                                source: dashboard.musicArtUrl
                                fillMode: Image.PreserveAspectCrop
                                visible: dashboard.musicArtUrl.length > 0
                                smooth: true
                                asynchronous: true
                            }

                            Text {
                                anchors.centerIn: parent
                                text: "󰎆"
                                visible: dashboard.musicArtUrl.length === 0
                                color: Theme.inactiveWs
                                font.family: Theme.font
                                font.pixelSize: 36
                            }
                        }

                        Text {
                            text: dashboard.musicTitle
                            color: Theme.text
                            font.family: Theme.font
                            font.pixelSize: 12
                            font.bold: true
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                            horizontalAlignment: Text.AlignHCenter
                        }

                        Text {
                            text: dashboard.musicArtist
                            color: Theme.defaultWs
                            font.family: Theme.font
                            font.pixelSize: 11
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                            horizontalAlignment: Text.AlignHCenter
                        }

                        Text {
                            text: dashboard.musicAlbum
                            color: Theme.inactiveWs
                            font.family: Theme.font
                            font.pixelSize: 10
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                            horizontalAlignment: Text.AlignHCenter
                        }

                        Text {
                            text: dashboard.musicPosition + " / " + dashboard.musicLength
                            color: Theme.text
                            font.family: Theme.font
                            font.pixelSize: 10
                            horizontalAlignment: Text.AlignHCenter
                            Layout.fillWidth: true
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            Layout.alignment: Qt.AlignHCenter
                            spacing: 10

                            Rectangle {
                                Layout.preferredWidth: 56
                                Layout.preferredHeight: 40
                                radius: 8
                                color: Theme.background
                                border.width: 1
                                border.color: Theme.grey

                                Text {
                                    anchors.centerIn: parent
                                    text: ""
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 16
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: dashboard.sendPlayerAction("previous")
                                }
                            }

                            Rectangle {
                                Layout.preferredWidth: 72
                                Layout.preferredHeight: 40
                                radius: 8
                                color: Theme.background
                                border.width: 1
                                border.color: Theme.grey

                                Text {
                                    anchors.centerIn: parent
                                    text: dashboard.musicStatus === "Playing" ? "" : ""
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 16
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: dashboard.sendPlayerAction("play-pause")
                                }
                            }

                            Rectangle {
                                Layout.preferredWidth: 56
                                Layout.preferredHeight: 40
                                radius: 8
                                color: Theme.background
                                border.width: 1
                                border.color: Theme.grey

                                Text {
                                    anchors.centerIn: parent
                                    text: ""
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 16
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: dashboard.sendPlayerAction("next")
                                }
                            }
                        }

                    }
                }
            }
        }
    }

    ParallelAnimation {
        id: calendarSwipeAnimation

        NumberAnimation {
            target: dashboard
            property: "calendarSwipeX"
            to: 0
            duration: 180
            easing.type: Easing.OutCubic
        }

        NumberAnimation {
            target: calendarDatesAnimatedLayer
            property: "opacity"
            to: 1
            duration: 180
            easing.type: Easing.OutCubic
        }
    }
}
