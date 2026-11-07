import QtQuick

// Media Extractor workspace \u2014 extracted from main.qml (Phase 2 split #3)
Rectangle {
    id: media

    color: Theme.bg

    // Re-check whenever the tab is shown: the first check can run before the daemon is up.
    onVisibleChanged: if (visible) media.onLoadFfmpegStatus()

    // Data bindings
    property string mediaUrl: ""
    property var mediaResult: null
    property string mediaSelectedFormat: ""
    property bool mediaLoading: false
    property string mediaError: ""
    property var mergeJobs: []
    property string ffmpegStatus: ""
    property string cookiesText: ""
    property string cookiesStatus: ""

    // Functions
    property var onExtract: function() {}
    property var onPlaylistDownload: function() {}
    property var onMediaDownload: function() {}
    property var onParseCookies: function() {}
    property var onExportCookies: function() {}
    property var onLoadMergeJobs: function() {}
    property var onLoadFfmpegStatus: function() {}
    property var onSetMediaUrl: function(url) {}
    property var onSetCookiesText: function(text) {}
    property var onSetMediaSelectedFormat: function(id) {}

    property var bus: null
    property var requestBus: null

    Column {
        anchors.fill: parent
        anchors.margins: 12
        spacing: 10

        Row {
            spacing: 8
            Text { text: qsTr("Media Extractor"); color: Theme.text; font.pixelSize: 14; font.bold: true }
            Rectangle {
                width: 80; height: 18; radius: 3; color: Theme.bg
                border.color: Theme.surfaceAlt; border.width: 1
                Text {
                    id: ytdlpLabel
                    anchors.centerIn: parent
                    font.pixelSize: 9; color: Theme.textDim; text: "yt-dlp ..."
                    property int tries: 0
                    function load() {
                        if (!media.requestBus) return;
                        media.requestBus("ytdlp_status", {}, function(raw) {
                            try {
                                var s = JSON.parse(raw);
                                if (s.error) throw s.error;
                                var ok = s.ytdlp && s.ytdlp.available;
                                text = ok ? "yt-dlp ✓" : "yt-dlp ✗";
                                color = ok ? Theme.success : Theme.danger;
                            } catch(e) {
                                // Cold start: the daemon may not be up yet; retry a few times.
                                text = "yt-dlp ?";
                                if (ytdlpLabel.tries++ < 6) ytdlpRetry.start();
                            }
                        });
                    }
                    Timer { id: ytdlpRetry; interval: 3000; onTriggered: ytdlpLabel.load() }
                    Component.onCompleted: load()
                }
            }
            Rectangle {
                width: 80; height: 18; radius: 3; color: Theme.bg
                border.color: Theme.surfaceAlt; border.width: 1
                Text {
                    anchors.centerIn: parent
                    font.pixelSize: 9
                    // Bound to the parent's status so it updates when the async check returns.
                    text: media.ffmpegStatus
                    color: media.ffmpegStatus.indexOf("✓") >= 0 ? Theme.success : Theme.danger
                    Component.onCompleted: media.onLoadFfmpegStatus()
                }
            }
        }

        // URL input
        Row {
            width: parent.width; spacing: 6
            Rectangle {
                // 90 (Extract) + 110 (Playlist DL) + 2 gaps of 6; the row used to overflow by ~110 px.
                width: parent.width - 212; height: 28; color: Theme.bg
                border.color: mediaUrlInput.activeFocus ? Theme.accent : Theme.surfaceAlt; radius: 3
                TextInput {
                    clip: true
                    id: mediaUrlInput
                    anchors { fill: parent; margins: 6 }
                    color: Theme.text; font.pixelSize: 12
                    Text { visible: parent.text.length === 0; text: qsTr("https://youtube.com/watch?v=..."); color: Theme.textDim; font.pixelSize: 12 }
                    onAccepted: media.onExtract()
                    onTextChanged: media.onSetMediaUrl(text)
                }
            }
            Rectangle {
                width: 90; height: 28; radius: 3
                color: mediaExtractBtnArea.containsMouse ? Theme.accent : Theme.accentDark
                Text { anchors.centerIn: parent; text: media.mediaLoading ? "..." : "Extract"; color: "white"; font.pixelSize: 12 }
                MouseArea { id: mediaExtractBtnArea; anchors.fill: parent; hoverEnabled: true; onClicked: media.onExtract() }
            }
            Rectangle {
                width: 110; height: 28; radius: 3
                color: mediaPlaylistBtnArea.containsMouse ? Theme.surfaceAlt : Theme.surface
                Text { anchors.centerIn: parent; text: qsTr("Playlist DL"); color: Theme.textDim; font.pixelSize: 12 }
                MouseArea { id: mediaPlaylistBtnArea; anchors.fill: parent; hoverEnabled: true; onClicked: media.onPlaylistDownload() }
            }
        }

        // Error
        Text {
            visible: media.mediaError !== ""
            width: parent.width
            text: media.mediaError
            color: media.mediaError.indexOf("started") >= 0 ? Theme.success : Theme.danger
            font.pixelSize: 11
            wrapMode: Text.Wrap
        }

        // Results
        Column {
            width: parent.width
            // leave room for the title row, merge jobs and the cookies box below
            height: Math.max(200, parent.height - 200)
            visible: media.mediaResult !== null
            spacing: 8

            // Video info
            Rectangle {
                width: parent.width; height: 50; radius: 4; color: Theme.bg
                Row {
                    anchors { fill: parent; margins: 8 }
                    spacing: 12
                    Column {
                        spacing: 2
                        Text { text: media.mediaResult ? media.mediaResult.title || "(untitled)" : ""; color: Theme.text; font.pixelSize: 12; elide: Text.ElideRight; width: 300 }
                        Text {
                            color: Theme.textDim; font.pixelSize: 10
                            text: {
                                if (!media.mediaResult) return "";
                                var r = media.mediaResult;
                                var parts = [];
                                if (r.uploader) parts.push(r.uploader);
                                if (r.duration_secs) parts.push(Math.round(r.duration_secs / 60) + " min");
                                if (r.extractor) parts.push(r.extractor);
                                if (r.formats) parts.push(r.formats.length + " formats");
                                return parts.join(" \u00b7 ");
                            }
                        }
                    }
                }
            }

            // Format list header
            Row {
                width: parent.width; spacing: 0
                Text { width: 30; text: qsTr("#"); color: Theme.textDim; font.pixelSize: 10 }
                Text { width: 24; text: qsTr(""); color: Theme.textDim; font.pixelSize: 10 }
                Text { width: 60; text: qsTr("ID"); color: Theme.textDim; font.pixelSize: 10 }
                Text { width: 200; text: qsTr("Label"); color: Theme.textDim; font.pixelSize: 10 }
                Text { width: 60; text: qsTr("Ext"); color: Theme.textDim; font.pixelSize: 10 }
                Text { width: 80; text: qsTr("Size"); color: Theme.textDim; font.pixelSize: 10 }
            }

            Rectangle { width: parent.width; height: 1; color: Theme.surfaceAlt }

            // Format list
            ListView {
                width: parent.width
                height: parent.height - 135
                clip: true
                cacheBuffer: 200
                model: media.mediaResult ? media.mediaResult.formats : []
                delegate: Rectangle {
                    width: ListView.view.width
                    height: 20
                    color: media.mediaSelectedFormat === modelData.id ? Theme.surfaceAlt : "transparent"
                    Row {
                        anchors.fill: parent
                        Text { width: 30; text: (index + 1); color: Theme.textDim; font.pixelSize: 10 }
                        Rectangle {
                            width: 10; height: 10; radius: 5
                            color: media.mediaSelectedFormat === modelData.id ? Theme.accent : Theme.bg
                            border.color: Theme.surfaceAlt; border.width: 1
                        }
                        Item { width: 14; height: 1 }
                        Text { width: 60; text: modelData.id; color: Theme.textDim; font.pixelSize: 10; elide: Text.ElideRight }
                        Text { width: 200; text: modelData.label || "-"; color: Theme.text; font.pixelSize: 10; elide: Text.ElideRight }
                        Text { width: 60; text: modelData.ext || "-"; color: Theme.textDim; font.pixelSize: 10 }
                        Text {
                            width: 80
                            color: Theme.textDim; font.pixelSize: 10
                            text: modelData.filesize ? (modelData.filesize / (1024*1024)).toFixed(1) + " MB" : "-"
                        }
                    }
                    MouseArea { anchors.fill: parent; onClicked: media.onSetMediaSelectedFormat(modelData.id) }
                }
            }

            // Download button
            Rectangle {
                width: 140; height: 26; radius: 3
                color: media.mediaSelectedFormat !== "" ? Theme.accent : Theme.surfaceAlt
                opacity: media.mediaSelectedFormat !== "" ? 1.0 : 0.4
                Text { anchors.centerIn: parent; text: qsTr("Download Selected"); color: "white"; font.pixelSize: 11 }
                MouseArea { anchors.fill: parent; enabled: media.mediaSelectedFormat !== ""; onClicked: media.onMediaDownload() }
            }
        }

        // Placeholder when no result
        Text {
            visible: media.mediaResult === null && !media.mediaLoading && media.mediaError === ""
            width: parent.width; horizontalAlignment: Text.AlignHCenter; topPadding: 40
            text: qsTr("Paste a video URL and click Extract")
            color: Theme.textDim; font.pixelSize: 12
        }

        // FQT-042: Merge jobs section
        Rectangle {
            width: parent.width; height: 1; color: Theme.surfaceAlt
            visible: media.mergeJobs.length > 0
        }
        Column {
            width: parent.width
            visible: media.mergeJobs.length > 0
            spacing: 4
            Row {
                spacing: 6
                Text { text: qsTr("Merge Jobs"); color: Theme.accent; font.pixelSize: 11; font.bold: true }
                Rectangle {
                    width: 50; height: 16; radius: 3; color: mergeRefreshArea.containsMouse ? Theme.surfaceAlt : "transparent"
                    Text { anchors.centerIn: parent; text: qsTr("Refresh"); color: Theme.textDim; font.pixelSize: 9 }
                    MouseArea { id: mergeRefreshArea; anchors.fill: parent; hoverEnabled: true; onClicked: media.onLoadMergeJobs() }
                }
            }
            Repeater {
                model: media.mergeJobs
                Row {
                    spacing: 8; height: 18
                    Rectangle {
                        width: 6; height: 6; radius: 3
                        color: modelData.state === "completed" ? Theme.success : modelData.state === "failed" ? Theme.danger : Theme.warning
                    }
                    Text { text: modelData.state || "-"; color: Theme.textDim; font.pixelSize: 10; width: 60 }
                    Text { text: modelData.message || "-"; color: Theme.textDim; font.pixelSize: 10; elide: Text.ElideRight; width: 200 }
                }
            }
        }

        // FQT-043: Cookies section
        Rectangle { width: parent.width; height: 1; color: Theme.surfaceAlt }

        Row {
            spacing: 6
            Text { text: qsTr("Cookies"); color: Theme.accent; font.pixelSize: 11; font.bold: true }
            Rectangle {
                width: 50; height: 18; radius: 3; color: cookiesParseArea.containsMouse ? Theme.surfaceAlt : "transparent"
                Text { anchors.centerIn: parent; text: qsTr("Parse"); color: Theme.textDim; font.pixelSize: 9 }
                MouseArea { id: cookiesParseArea; anchors.fill: parent; hoverEnabled: true; onClicked: media.onParseCookies() }
            }
            Rectangle {
                width: 50; height: 18; radius: 3; color: cookiesExportArea.containsMouse ? Theme.surfaceAlt : "transparent"
                Text { anchors.centerIn: parent; text: qsTr("Export"); color: Theme.textDim; font.pixelSize: 9 }
                MouseArea { id: cookiesExportArea; anchors.fill: parent; hoverEnabled: true; onClicked: media.onExportCookies() }
            }
            Text {
                text: media.cookiesStatus
                color: media.cookiesStatus.indexOf("Error") >= 0 ? Theme.danger : Theme.success
                font.pixelSize: 9
            }
        }

        Rectangle {
            width: parent.width; height: 60; radius: 3; color: Theme.bg
            border.color: cookiesTextArea.activeFocus ? Theme.accent : Theme.surfaceAlt
            clip: true
            TextEdit {
                id: cookiesTextArea
                anchors { fill: parent; margins: 6 }
                color: Theme.text; font.pixelSize: 10; font.family: "monospace"
                wrapMode: TextEdit.NoWrap
                text: media.cookiesText
                onTextChanged: media.onSetCookiesText(text)
            }
        }

        Text {
            width: parent.width
            text: qsTr("Paste Netscape cookies.txt content. Browser cookie DB access is disabled for privacy \u2014 use the extension capture path or paste manually.")
            color: Theme.textDim; font.pixelSize: 9; wrapMode: Text.Wrap
        }
    }
}
