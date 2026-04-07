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
    property var calendarCache: ({})
    property var calendarPrefetchQueue: []
    property bool calendarPrefetchRunning: false
    property int calendarPrefetchYear: 0
    property int calendarPrefetchMonth: 0
    readonly property int dashboardRefreshMs: 2000

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

    function parseGpuPercentValue(textValue, numericValue) {
        const gpuValue = Number(numericValue);
        if (!isNaN(gpuValue))
            return clampPercent(String(Math.round(gpuValue)));

        const parsedText = parseInt(String(textValue || "N/A"));
        return isNaN(parsedText) ? 0 : clampPercent(String(parsedText));
    }

    function applyMusicPayload(musicPayload) {
        const music = (musicPayload && typeof musicPayload === "object") ? musicPayload : {};
        musicStatus = String(music.status || "Unknown").trim();
        musicPlayer = String(music.player || "N/A").trim();
        musicTitle = String(music.title || "Nothing playing").trim();
        musicArtist = String(music.artist || "N/A").trim();
        musicAlbum = String(music.album || "N/A").trim();
        musicPosition = String(music.position || "00:00").trim();
        musicLength = String(music.length || "00:00").trim();
        musicArtUrl = String(music.art_url || "").trim();
        musicPlayerTitle = String(music.player_title || musicPlayer || "N/A").trim();
    }

    function applyPerformancePayload(performancePayload) {
        const performance = (performancePayload && typeof performancePayload === "object") ? performancePayload : {};
        cpuPercent = clampPercent(String(performance.cpu_percent || "0"));

        gpuPercentText = String(performance.gpu_percent_text || "N/A").trim();
        gpuPercentValue = parseGpuPercentValue(gpuPercentText, performance.gpu_percent_value);
        gpuSource = String(performance.gpu_source || "N/A").trim();

        ramUsedGiB = parseNumber(performance.ram_used_gib, 0);
        ramTotalGiB = parseNumber(performance.ram_total_gib, 0);
        ramPercent = clampPercent(String(performance.ram_percent || "0"));

        storageUsedGiB = parseNumber(performance.storage_used_gib, 0);
        storageTotalGiB = parseNumber(performance.storage_total_gib, 0);
        storagePercent = clampPercent(String(performance.storage_percent || "0"));
    }

    function applyDashboardResponse(response, raw) {
        const calendarPayload = parseCalendarPayload(raw);
        if (calendarPayload.hasCalendar) {
            cacheCalendarPayload(calendarPayload);

            // Avoid visual jump: only apply responses for the currently selected month.
            if (calendarPayload.year === selectedCalendarYear && calendarPayload.month === selectedCalendarMonth) {
                applyCalendarPayload(calendarPayload, false);
                preloadNearbyCalendars(calendarPayload.year, calendarPayload.month);
            }
        }

        applyMusicPayload(response.music);
        applyPerformancePayload(response.performance);
        lastError = "";
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
        interval: dashboard.dashboardRefreshMs
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

                dashboard.applyDashboardResponse(response, raw);
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

                DashboardPerformancePanel {
                    id: performancePanelItem
                    Layout.alignment: Qt.AlignTop
                    Layout.preferredWidth: 270
                    Layout.preferredHeight: Math.max(performancePanelItem.implicitHeight, musicPanelItem.implicitHeight)
                    Layout.minimumHeight: Math.max(performancePanelItem.implicitHeight, musicPanelItem.implicitHeight)
                    Layout.maximumHeight: Math.max(performancePanelItem.implicitHeight, musicPanelItem.implicitHeight)
                    cpuPercent: dashboard.cpuPercent
                    gpuPercentText: dashboard.gpuPercentText
                    gpuPercentValue: dashboard.gpuPercentValue
                    gpuSource: dashboard.gpuSource
                    ramUsedGiB: dashboard.ramUsedGiB
                    ramTotalGiB: dashboard.ramTotalGiB
                    ramPercent: dashboard.ramPercent
                    storageUsedGiB: dashboard.storageUsedGiB
                    storageTotalGiB: dashboard.storageTotalGiB
                    storagePercent: dashboard.storagePercent
                }

                DashboardCalendarPanel {
                    id: calendarPanelItem
                    Layout.alignment: Qt.AlignTop
                    Layout.preferredWidth: dashboard.calendarGridWidth + 8
                    Layout.minimumWidth: dashboard.calendarGridWidth + 8
                    Layout.maximumWidth: dashboard.calendarGridWidth + 8
                    Layout.preferredHeight: Math.max(performancePanelItem.implicitHeight, musicPanelItem.implicitHeight)
                    Layout.minimumHeight: Math.max(performancePanelItem.implicitHeight, musicPanelItem.implicitHeight)
                    Layout.maximumHeight: Math.max(performancePanelItem.implicitHeight, musicPanelItem.implicitHeight)
                    title: dashboard.calendarTitle.length > 0 ? dashboard.calendarTitle : "Calendar"
                    calendarWeekdays: dashboard.calendarWeekdays
                    calendarCellWidth: dashboard.calendarCellWidth
                    calendarCellHeight: dashboard.calendarCellHeight
                    calendarGridGap: dashboard.calendarGridGap
                    calendarWeekdayHeight: dashboard.calendarWeekdayHeight
                    calendarWeekdayGap: dashboard.calendarWeekdayGap
                    calendarWeekdayToDatesGap: dashboard.calendarWeekdayToDatesGap
                    gridDayValueFn: function(index) {
                        return dashboard.gridDayValue(index);
                    }
                    gridCellCurrentMonthFn: function(index) {
                        return dashboard.gridCellCurrentMonth(index);
                    }
                    gridCellIsTodayFn: function(index) {
                        return dashboard.gridCellIsToday(index);
                    }
                    weekdayLabelFn: function(label) {
                        return dashboard.weekdayLabel(label);
                    }
                    onPreviousYearRequested: dashboard.changeCalendarYear(-1)
                    onPreviousMonthRequested: dashboard.changeCalendarMonth(-1)
                    onTodayRequested: dashboard.jumpCalendarToToday()
                    onNextMonthRequested: dashboard.changeCalendarMonth(1)
                    onNextYearRequested: dashboard.changeCalendarYear(1)
                }

                DashboardMusicPanel {
                    id: musicPanelItem
                    Layout.alignment: Qt.AlignTop
                    Layout.preferredWidth: 310
                    Layout.preferredHeight: Math.max(performancePanelItem.implicitHeight, musicPanelItem.implicitHeight)
                    Layout.minimumHeight: Math.max(performancePanelItem.implicitHeight, musicPanelItem.implicitHeight)
                    Layout.maximumHeight: Math.max(performancePanelItem.implicitHeight, musicPanelItem.implicitHeight)
                    panelVisible: dashboard.visible
                    musicStatus: dashboard.musicStatus
                    musicTitle: dashboard.musicTitle
                    musicArtist: dashboard.musicArtist
                    musicAlbum: dashboard.musicAlbum
                    musicPosition: dashboard.musicPosition
                    musicLength: dashboard.musicLength
                    musicArtUrl: dashboard.musicArtUrl
                    onPreviousRequested: dashboard.sendPlayerAction("previous")
                    onPlayPauseRequested: dashboard.sendPlayerAction("play-pause")
                    onNextRequested: dashboard.sendPlayerAction("next")
                }
            }
        }
    }

}
