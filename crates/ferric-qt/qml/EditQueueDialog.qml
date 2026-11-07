import QtQuick

// FQT-045: Edit Queue dialog \u2014 extracted from main.qml
AppDialog {
    id: editQueueDialog

    property var doSaveQueue
    // Weekday bits like the daemon's sched_days: bit 0 = Sunday ... bit 6 = Saturday.
    property int daysMask: 127
    property string whenDone: ""
    // IDM's Scheduler shows every queue; with more than one, a row of names switches between them.
    property var queueList: []
    property string currentQueueId: ""
    property var onPickQueue: function(q) {}

    cardMaxHeight: 620
    title: qsTr("Edit Queue")

    function populate(q) {
        editQueueName.text = q.name || "";
        editQueueMaxConc.text = (q.max_concurrent || 1).toString();
        editQueueEnabled.checked = !!q.enabled;
        editQueueStartup.checked = !!q.start_on_startup;
        editQueueSync.text = q.sync_interval_min ? q.sync_interval_min.toString() : "";
        editQueueRetries.text = (q.max_retries === null || q.max_retries === undefined) ? "" : q.max_retries.toString();
        editQueueOpenFile.text = q.open_file || "";
        whenDone = q.when_done || "";
        editQueueSchedStart.text = minutesToClock(q.sched_start);
        editQueueSchedEnd.text = minutesToClock(q.sched_end);
        daysMask = q.sched_days === undefined ? 127 : q.sched_days;
    }

    // The daemon stores minutes from local midnight; the UI shows HH:MM like IDM's scheduler.
    function minutesToClock(m) {
        if (m === null || m === undefined || m === "") return "";
        var h = Math.floor(m / 60), mm = m % 60;
        return (h < 10 ? "0" : "") + h + ":" + (mm < 10 ? "0" : "") + mm;
    }

    // "" = no limit; invalid input yields "" too so a typo never schedules the wrong hour.
    function clockToMinutes(text) {
        var m = /^\s*(\d{1,2}):(\d{2})\s*$/.exec(text);
        if (!m) return "";
        var h = parseInt(m[1]), mi = parseInt(m[2]);
        if (h > 23 || mi > 59) return "";
        return (h * 60 + mi).toString();
    }

    function readFields() {
        return {
            name: editQueueName.text.trim(),
            max_concurrent: parseInt(editQueueMaxConc.text) || 1,
            enabled: editQueueEnabled.checked,
            sched_start: clockToMinutes(editQueueSchedStart.text),
            sched_end: clockToMinutes(editQueueSchedEnd.text),
            sched_days: daysMask,
            start_on_startup: editQueueStartup.checked,
            sync_interval_min: parseInt(editQueueSync.text) > 0 ? parseInt(editQueueSync.text) : null,
            max_retries: editQueueRetries.text.trim() === "" ? null : parseInt(editQueueRetries.text),
            open_file: editQueueOpenFile.text.trim() === "" ? null : editQueueOpenFile.text.trim(),
            when_done: whenDone === "" ? null : whenDone
        };
    }

    contentItem: [
        Row {
            visible: editQueueDialog.queueList.length > 1 && editQueueDialog.currentQueueId !== ""
            spacing: Theme.sp1; height: visible ? 26 : 0
            Repeater {
                model: editQueueDialog.queueList
                Rectangle {
                    width: qLabel.implicitWidth + 16; height: 24; radius: 3
                    color: modelData.id === editQueueDialog.currentQueueId ? Theme.accent : Theme.surface
                    Text { id: qLabel; anchors.centerIn: parent; text: modelData.name; font.pixelSize: Theme.fontSizeSmall
                        color: modelData.id === editQueueDialog.currentQueueId ? "white" : Theme.text }
                    MouseArea { anchors.fill: parent; onClicked: editQueueDialog.onPickQueue(modelData) }
                }
            }
        },

        Column { width: parent.width; spacing: Theme.sp1
            Text { text: qsTr("Name"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Rectangle {
                width: parent.width; height: 28; color: Theme.bg
                border.color: editQueueName.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                TextInput {
                    clip: true
                    id: editQueueName
                    anchors { fill: parent; margins: Theme.sp1 }
                    color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                }
            }
        },

        Column { width: parent.width; spacing: Theme.sp1
            Text { text: qsTr("Max concurrent downloads"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Rectangle {
                width: parent.width; height: 28; color: Theme.bg
                border.color: editQueueMaxConc.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                TextInput {
                    clip: true
                    id: editQueueMaxConc
                    anchors { fill: parent; margins: Theme.sp1 }
                    color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                    inputMethodHints: Qt.ImhDigitsOnly
                }
            }
        },

        Row {
            spacing: Theme.sp2; height: 22
            Rectangle {
                id: editQueueEnabled
                property bool checked: true
                width: 16; height: 16; radius: 3
                color: checked ? Theme.accent : Theme.bg
                border.color: Theme.surfaceAlt; border.width: 1
                Text { anchors.centerIn: parent; text: parent.checked ? "\u2713" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { anchors.fill: parent; onClicked: parent.checked = !parent.checked }
            }
            Text { text: qsTr("Enabled"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
        },

        Row {
            spacing: Theme.sp2; height: 22
            Rectangle {
                id: editQueueStartup
                property bool checked: false
                width: 16; height: 16; radius: 3
                color: checked ? Theme.accent : Theme.bg
                border.color: Theme.surfaceAlt; border.width: 1
                Text { anchors.centerIn: parent; text: parent.checked ? "✓" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { anchors.fill: parent; onClicked: parent.checked = !parent.checked }
            }
            Text { text: qsTr("Start download on Ferric startup"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
        },

        Row {
            spacing: Theme.sp2; height: 28
            Text { text: qsTr("Retries for each file (empty = global):"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; anchors.verticalCenter: parent.verticalCenter }
            Rectangle {
                width: 60; height: 26; color: Theme.bg; radius: 3
                border.color: editQueueRetries.activeFocus ? Theme.accent : Theme.surfaceAlt
                TextInput { id: editQueueRetries; clip: true; anchors { fill: parent; margins: Theme.sp1 }
                    color: Theme.text; font.pixelSize: Theme.fontSizeNormal; inputMethodHints: Qt.ImhDigitsOnly }
            }
        },

        Column { width: parent.width; spacing: Theme.sp1
            Text { text: qsTr("Open this file when the queue is done"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Rectangle {
                width: parent.width; height: 26; color: Theme.bg; radius: 3
                border.color: editQueueOpenFile.activeFocus ? Theme.accent : Theme.surfaceAlt
                TextInput { id: editQueueOpenFile; clip: true; anchors { fill: parent; margins: Theme.sp1 }
                    color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
            }
        },

        Row {
            spacing: Theme.sp1; height: 26
            Text { text: qsTr("When done:"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; anchors.verticalCenter: parent.verticalCenter }
            Repeater {
                model: [ { id: "", label: qsTr("Nothing") }, { id: "exit", label: qsTr("Exit Ferric") }, { id: "shutdown", label: qsTr("Turn off computer") } ]
                Rectangle {
                    width: wdLabel.implicitWidth + 14; height: 24; radius: 3
                    color: editQueueDialog.whenDone === modelData.id ? Theme.accent : Theme.surface
                    Text { id: wdLabel; anchors.centerIn: parent; text: modelData.label; font.pixelSize: Theme.fontSizeSmall
                        color: editQueueDialog.whenDone === modelData.id ? "white" : Theme.text }
                    MouseArea { anchors.fill: parent; onClicked: editQueueDialog.whenDone = modelData.id }
                }
            }
        },

        Column { width: parent.width; spacing: Theme.sp1
            Text { text: qsTr("Periodic synchronization: check finished files every N minutes (empty = off)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.WordWrap; width: parent.width }
            Rectangle {
                width: 140; height: 28; color: Theme.bg
                border.color: editQueueSync.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                TextInput {
                    clip: true
                    id: editQueueSync
                    anchors { fill: parent; margins: Theme.sp1 }
                    color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                    inputMethodHints: Qt.ImhDigitsOnly
                }
            }
        },

        Row {
            spacing: Theme.sp2
            Column { width: 140; spacing: Theme.sp1
                Text { text: qsTr("Start time (HH:MM)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle {
                    width: parent.width; height: 28; color: Theme.bg
                    border.color: editQueueSchedStart.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                    TextInput {
                        clip: true
                        id: editQueueSchedStart
                        anchors { fill: parent; margins: Theme.sp1 }
                        color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                                    }
                }
            }
            Column { width: 140; spacing: Theme.sp1
                Text { text: qsTr("End time (HH:MM)"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                Rectangle {
                    width: parent.width; height: 28; color: Theme.bg
                    border.color: editQueueSchedEnd.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                    TextInput {
                        clip: true
                        id: editQueueSchedEnd
                        anchors { fill: parent; margins: Theme.sp1 }
                        color: Theme.text; font.pixelSize: Theme.fontSizeNormal
                    }
                }
            }
        },

        Row {
            spacing: Theme.sp1
            Repeater {
                model: [qsTr("Sun"), qsTr("Mon"), qsTr("Tue"), qsTr("Wed"), qsTr("Thu"), qsTr("Fri"), qsTr("Sat")]
                Rectangle {
                    property bool on: (editQueueDialog.daysMask & (1 << index)) !== 0
                    width: 40; height: 24; radius: 3
                    color: on ? Theme.accent : Theme.bg
                    border.color: Theme.surfaceAlt; border.width: 1
                    Text { anchors.centerIn: parent; text: modelData; color: parent.on ? "white" : Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    MouseArea { anchors.fill: parent; onClicked: editQueueDialog.daysMask = editQueueDialog.daysMask ^ (1 << index) }
                }
            }
        },

        Text { text: qsTr("Blank times = always active. Use 24-hour times such as 22:30. Days: the queue only runs on the highlighted days."); width: parent.width; wrapMode: Text.Wrap; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
    ]

    buttons: [
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.bg
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; onClicked: editQueueDialog.close() }
        },
        Rectangle {
            width: Theme.btnM; height: Theme.controlH; radius: Theme.radius; color: Theme.accent
            Text { anchors.centerIn: parent; text: qsTr("Save"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
            MouseArea { anchors.fill: parent; onClicked: if (doSaveQueue) doSaveQueue() }
        }
    ]
}
