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

    function parseCliJson(raw) {
        const text = String(raw || "").trim();
        if (!text.length)
            return null;

        try {
            return JSON.parse(text);
        } catch (_error) {
            return null;
        }
    }

    function clampPercent(value) {
        const n = parseInt(value);
        if (isNaN(n))
            return 0;
        return Math.max(0, Math.min(100, n));
    }

    function metricColor(percent) {
        if (percent >= 85)
            return Theme.error;
        if (percent >= 65)
            return Theme.tertiary;
        return Theme.secondary;
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

        const parsed = parseCliJson(rawText);
        if (!parsed || parsed.ok !== true)
            return payload;

        const calendar = parsed.calendar || null;
        if (!calendar || typeof calendar !== "object")
            return payload;

        payload.title = String(calendar.title || payload.title).trim();

        const year = parseInt(String(calendar.year));
        const month = parseInt(String(calendar.month));
        const daysInMonth = parseInt(String(calendar.days_in_month));
        const firstWeekday = parseInt(String(calendar.first_weekday));
        const today = parseInt(String(calendar.today));

        payload.year = isNaN(year) ? payload.year : year;
        payload.month = isNaN(month) ? payload.month : month;
        payload.daysInMonth = isNaN(daysInMonth) ? payload.daysInMonth : Math.max(1, daysInMonth);
        payload.firstWeekday = isNaN(firstWeekday) ? payload.firstWeekday : Math.max(0, Math.min(6, firstWeekday));
        payload.today = isNaN(today) ? payload.today : today;

        const defaultWeekdays = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
        const weekdays = Array.isArray(calendar.weekdays) ? calendar.weekdays : [];
        payload.weekdays = [];
        for (let i = 0; i < 7; i++)
            payload.weekdays.push(String(weekdays[i] || defaultWeekdays[i]).trim());

        const rows = createEmptyCalendarRows();
        const inputRows = Array.isArray(calendar.rows) ? calendar.rows : [];
        for (let r = 0; r < 6; r++) {
            const rowData = Array.isArray(inputRows[r]) ? inputRows[r] : [];
            for (let c = 0; c < 7; c++) {
                const day = parseInt(String(rowData[c] || "0"));
                rows[r][c] = isNaN(day) ? 0 : day;
            }
        }
        payload.rows = rows;
        payload.hasCalendar = true;

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
            "stratum-cli",
            "dashboard",
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
            "stratum-cli",
            "dashboard",
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
        command: ["stratum-cli", "dashboard", "all", String(selectedCalendarYear), String(selectedCalendarMonth)]
        running: false
        stdout: StdioCollector {
            onStreamFinished: {
                dashboard.loading = false;

                const raw = this.text.trim();
                if (!raw) {
                    dashboard.lastError = "No dashboard data";
                    return;
                }

                const response = dashboard.parseCliJson(raw);
                if (!response) {
                    dashboard.lastError = "Invalid dashboard response";
                    return;
                }
                if (response.ok !== true) {
                    dashboard.lastError = String(response.error || "Dashboard command failed");
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

                const music = (response.music && typeof response.music === "object") ? response.music : {};
                dashboard.musicStatus = String(music.status || "Unknown").trim();
                dashboard.musicPlayer = String(music.player || "N/A").trim();
                dashboard.musicTitle = String(music.title || "Nothing playing").trim();
                dashboard.musicArtist = String(music.artist || "N/A").trim();
                dashboard.musicAlbum = String(music.album || "N/A").trim();
                dashboard.musicPosition = String(music.position || "00:00").trim();
                dashboard.musicLength = String(music.length || "00:00").trim();
                dashboard.musicArtUrl = String(music.art_url || "").trim();
                dashboard.musicPlayerTitle = String(music.player_title || dashboard.musicPlayer || "N/A").trim();

                const performance = (response.performance && typeof response.performance === "object") ? response.performance : {};
                dashboard.cpuPercent = dashboard.clampPercent(String(performance.cpu_percent || "0"));

                dashboard.gpuPercentText = String(performance.gpu_percent_text || "N/A").trim();
                const gpuValue = Number(performance.gpu_percent_value);
                if (!isNaN(gpuValue))
                    dashboard.gpuPercentValue = dashboard.clampPercent(String(Math.round(gpuValue)));
                else {
                    const parsedGpu = parseInt(dashboard.gpuPercentText);
                    dashboard.gpuPercentValue = isNaN(parsedGpu) ? 0 : dashboard.clampPercent(String(parsedGpu));
                }
                dashboard.gpuSource = String(performance.gpu_source || "N/A").trim();

                dashboard.ramUsedGiB = dashboard.parseNumber(performance.ram_used_gib, 0);
                dashboard.ramTotalGiB = dashboard.parseNumber(performance.ram_total_gib, 0);
                dashboard.ramPercent = dashboard.clampPercent(String(performance.ram_percent || "0"));

                dashboard.storageUsedGiB = dashboard.parseNumber(performance.storage_used_gib, 0);
                dashboard.storageTotalGiB = dashboard.parseNumber(performance.storage_total_gib, 0);
                dashboard.storagePercent = dashboard.clampPercent(String(performance.storage_percent || "0"));

                dashboard.lastError = "";
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
        border.color: Theme.outlineVariant

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
                    color: Theme.error
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
                    border.color: Theme.outlineVariant

                    ColumnLayout {
                        id: performanceColumn
                        anchors.fill: parent
                        anchors.margins: 10
                        spacing: 6

                        Text {
                            text: "Performance"
                            color: Theme.on_Background
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
                            border.color: Theme.outlineVariant

                            ColumnLayout {
                                anchors.fill: parent
                                anchors.margins: 8
                                spacing: 2

                                RowLayout {
                                    spacing: 4

                                    Text {
                                        text: ""
                                        color: Theme.primary
                                        font.family: Theme.font
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        text: "CPU"
                                        color: Theme.on_Background
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
                                    color: Theme.surfaceContainerLowest

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
                            border.color: Theme.outlineVariant

                            ColumnLayout {
                                anchors.fill: parent
                                anchors.margins: 8
                                spacing: 2

                                RowLayout {
                                    spacing: 4

                                    Text {
                                        text: "󰢮"
                                        color: Theme.primary
                                        font.family: Theme.font
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        text: "GPU"
                                        color: Theme.on_Background
                                        font.family: Theme.font
                                        font.pixelSize: 10
                                        font.bold: true
                                    }

                                    Text {
                                        text: "|"
                                        color: Theme.on_Background
                                        font.family: Theme.font
                                        font.pixelSize: 10
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        text: dashboard.gpuSource
                                        color: Theme.on_Background
                                        font.family: Theme.font
                                        font.pixelSize: 9
                                        elide: Text.ElideRight
                                        font.bold: true
                                    }
                                }

                                Text {
                                    text: dashboard.gpuPercentText === "N/A" ? "N/A" : dashboard.gpuPercentText + "%"
                                    color: dashboard.gpuPercentText === "N/A" ? Theme.on_Background : Theme.secondary
                                    font.family: Theme.font
                                    font.pixelSize: 17
                                    font.bold: true
                                }

                                Rectangle {
                                    Layout.fillWidth: true
                                    implicitHeight: 6
                                    radius: 3
                                    color: Theme.surfaceContainerLowest
                                    visible: dashboard.gpuPercentText !== "N/A"

                                    Rectangle {
                                        width: parent.width * dashboard.gpuPercentValue / 100
                                        height: parent.height
                                        radius: 3
                                        color: Theme.secondary

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
                            border.color: Theme.outlineVariant

                            ColumnLayout {
                                anchors.fill: parent
                                anchors.margins: 8
                                spacing: 2

                                RowLayout {
                                    spacing: 4

                                    Text {
                                        text: "󰍛"
                                        color: Theme.primary
                                        font.family: Theme.font
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        text: "RAM"
                                        color: Theme.on_Background
                                        font.family: Theme.font
                                        font.pixelSize: 10
                                        font.bold: true
                                    }
                                }

                                Text {
                                    text: dashboard.ramUsedGiB.toFixed(1) + " / " + dashboard.ramTotalGiB.toFixed(1) + " GiB"
                                    color: Theme.on_Surface
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
                                    color: Theme.surfaceContainerLowest

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
                            border.color: Theme.outlineVariant

                            ColumnLayout {
                                anchors.fill: parent
                                anchors.margins: 8
                                spacing: 2

                                RowLayout {
                                    spacing: 4

                                    Text {
                                        text: ""
                                        color: Theme.primary
                                        font.family: Theme.font
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        text: "Storage"
                                        color: Theme.on_Background
                                        font.family: Theme.font
                                        font.pixelSize: 10
                                        font.bold: true
                                    }
                                }

                                Text {
                                    text: dashboard.storageUsedGiB.toFixed(1) + " / " + dashboard.storageTotalGiB.toFixed(1) + " GiB"
                                    color: Theme.on_Surface
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
                                    color: Theme.surfaceContainerLowest

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
                    border.color: Theme.outlineVariant

                    ColumnLayout {
                        id: calendarColumn
                        anchors.fill: parent
                        anchors.margins: 4
                        spacing: 0

                        Text {
                            text: dashboard.calendarTitle.length > 0 ? dashboard.calendarTitle : "Calendar"
                            color: Theme.on_Surface
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

                                Rectangle {
                                    id: weekdaysBox
                                    width: (dashboard.calendarCellWidth) * 7 + (6 * 3)
                                    height: dashboard.calendarWeekdayHeight
                                    radius: 8
                                    color: Theme.background   
                                    clip: true                    

                                    Rectangle {
                                        anchors.left: parent.left
                                        anchors.right: parent.right
                                        anchors.bottom: parent.bottom
                                        height: 1
                                        color: Theme.outline
                                    }

                                    Row {
                                        anchors.fill: parent
                                        spacing: 3

                                        Repeater {
                                            model: dashboard.calendarWeekdays
                                            delegate: Rectangle {
                                                required property var modelData
                                                width: dashboard.calendarCellWidth
                                                height: dashboard.calendarWeekdayHeight
                                                color: "transparent"   // no per-item border
                                                // optional: add only internal separators if desired

                                                Text {
                                                    anchors.fill: parent
                                                    anchors.margins: 3
                                                    text: dashboard.weekdayLabel(modelData)
                                                    color: Theme.on_Background
                                                    font.family: Theme.font
                                                    font.pixelSize: 11
                                                    font.bold: true
                                                    horizontalAlignment: Text.AlignHCenter
                                                    verticalAlignment: Text.AlignVCenter
                                                }
                                            }
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
                                            color: isToday ? Theme.primary : Theme.background
                                            border.width: isToday ? 1 : 0
                                            border.color: Theme.primary

                                            Text {
                                                anchors.centerIn: parent
                                                text: String(dayValue)
                                                color: isToday ? Theme.background : (inCurrentMonth ? Theme.on_Surface : Theme.surfaceContainerLow)
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
                                border.color: Theme.outlineVariant

                                Text {
                                    anchors.centerIn: parent
                                    text: "<<"
                                    color: Theme.on_Surface
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
                                border.color: Theme.outlineVariant

                                Text {
                                    anchors.centerIn: parent
                                    text: "<"
                                    color: Theme.on_Surface
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
                                border.color: Theme.outlineVariant

                                Text {
                                    anchors.centerIn: parent
                                    text: "Today"
                                    color: Theme.primary
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
                                border.color: Theme.outlineVariant

                                Text {
                                    anchors.centerIn: parent
                                    text: ">"
                                    color: Theme.on_Surface
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
                                border.color: Theme.outlineVariant

                                Text {
                                    anchors.centerIn: parent
                                    text: ">>"
                                    color: Theme.on_Surface
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
                    border.color: Theme.outlineVariant

                    ColumnLayout {
                        id: musicColumn
                        anchors.fill: parent
                        anchors.margins: 10
                        spacing: 6

                        Text {
                            text: "Now Playing"
                            color: Theme.on_Surface
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
                            border.color: Theme.outlineVariant
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
                                color: Theme.surfaceContainerLow
                                font.family: Theme.font
                                font.pixelSize: 36
                            }
                        }

                        Item {
                            id: musicTitleClip
                            Layout.fillWidth: true
                            Layout.minimumHeight: musicTitleTextA.implicitHeight
                            implicitHeight: musicTitleTextA.implicitHeight
                            clip: true

                            property int marqueeGap: 24
                            property real scrollSpeed: 42
                            property bool titleOverflow: musicTitleTextA.implicitWidth > width
                            property real loopSpan: musicTitleTextA.implicitWidth + marqueeGap
                            property real tickerOffset: 0

                            Text {
                                id: musicTitleTextA
                                text: dashboard.musicTitle
                                color: Theme.on_Surface
                                font.family: Theme.font
                                font.pixelSize: 12
                                font.bold: true
                                elide: Text.ElideNone
                                wrapMode: Text.NoWrap
                                anchors.verticalCenter: parent.verticalCenter
                                x: musicTitleClip.titleOverflow ? musicTitleClip.tickerOffset : Math.round((musicTitleClip.width - implicitWidth) / 2)
                            }

                            Text {
                                id: musicTitleTextB
                                text: dashboard.musicTitle
                                color: Theme.on_Surface
                                font.family: Theme.font
                                font.pixelSize: 12
                                font.bold: true
                                elide: Text.ElideNone
                                wrapMode: Text.NoWrap
                                visible: musicTitleClip.titleOverflow
                                anchors.verticalCenter: parent.verticalCenter
                                x: musicTitleClip.tickerOffset + musicTitleClip.loopSpan
                            }

                            NumberAnimation {
                                id: musicTitleMarquee
                                target: musicTitleClip
                                property: "tickerOffset"
                                from: 0
                                to: -musicTitleClip.loopSpan
                                duration: Math.max(1, Math.round((musicTitleClip.loopSpan / musicTitleClip.scrollSpeed) * 1000))
                                easing.type: Easing.Linear
                                running: dashboard.visible && musicTitleClip.titleOverflow
                                loops: Animation.Infinite

                                onRunningChanged: {
                                    if (!running)
                                        musicTitleClip.tickerOffset = 0;
                                }
                            }

                            onTitleOverflowChanged: {
                                if (!titleOverflow)
                                    tickerOffset = 0;
                            }
                        }

                        Text {
                            text: dashboard.musicArtist
                            color: Theme.secondary
                            font.family: Theme.font
                            font.pixelSize: 11
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                            horizontalAlignment: Text.AlignHCenter
                        }

                        Text {
                            text: dashboard.musicAlbum
                            color: Theme.tertiary
                            font.family: Theme.font
                            font.pixelSize: 10
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                            horizontalAlignment: Text.AlignHCenter
                        }

                        Text {
                            text: dashboard.musicPosition + " / " + dashboard.musicLength
                            color: Theme.on_Surface
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
                                border.color: Theme.outlineVariant

                                Text {
                                    anchors.centerIn: parent
                                    text: ""
                                    color: Theme.on_Surface
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
                                border.color: Theme.outlineVariant

                                Text {
                                    anchors.centerIn: parent
                                    text: dashboard.musicStatus === "Playing" ? "" : ""
                                    color: Theme.on_Surface
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
                                border.color: Theme.outlineVariant

                                Text {
                                    anchors.centerIn: parent
                                    text: ""
                                    color: Theme.on_Surface
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
