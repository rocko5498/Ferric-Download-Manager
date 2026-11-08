import QtQuick
import QtQuick.Controls as QQC

// Media Extractor workspace: paste a video link, pick a format, download.
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

    // A small status chip: soft tinted background, dark text, so "missing" reads as a hint and not as an alarm.
    component Chip: Rectangle {
        property string label: ""
        property bool ok: false
        property bool known: true
        width: chipText.implicitWidth + 2 * Theme.sp3; height: Theme.sp4; radius: height / 2
        color: !known ? Theme.surfaceAlt : ok ? Theme.successSoft : Theme.warningSoft
        Text { id: chipText; anchors.centerIn: parent; text: parent.label; font.pixelSize: Theme.fontSizeSmall; font.weight: Font.DemiBold
            color: !parent.known ? Theme.textDim : parent.ok ? Theme.success : Theme.warningText }
    }

    // Plain text button inside a card.
    component TextAction: Rectangle {
        property string label: ""
        signal clicked()
        width: taText.implicitWidth + 2 * Theme.sp3; height: Theme.sp4 + Theme.sp1; radius: Theme.radius
        color: taArea.containsMouse ? Theme.surfaceAlt : "transparent"
        Text { id: taText; anchors.centerIn: parent; text: parent.label; color: Theme.accent; font.pixelSize: Theme.fontSizeSmall; font.weight: Font.DemiBold }
        MouseArea { id: taArea; anchors.fill: parent; hoverEnabled: true; onClicked: parent.clicked() }
    }

    Flickable {
        id: scroller
        anchors.fill: parent
        contentWidth: width
        contentHeight: mainCol.height + 2 * Theme.sp4
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        QQC.ScrollBar.vertical: QQC.ScrollBar {}

        Column {
            id: mainCol
            x: Theme.sp4; y: Theme.sp4
            width: scroller.width - 2 * Theme.sp4
            spacing: Theme.sp3

            // Title and tool status
            Row {
                spacing: Theme.sp3
                Column {
                    spacing: 2
                    Text { text: qsTr("Media Extractor"); color: Theme.text; font.pixelSize: Theme.fontSizeLarge; font.weight: Font.Bold }
                    Text { text: qsTr("Download a video or a whole playlist from a web page link."); color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal }
                }
            }
            Row {
                spacing: Theme.sp2
                Chip {
                    id: ytdlpChip
                    label: "yt-dlp"
                    known: false
                    property int tries: 0
                    function load() {
                        if (!media.requestBus) return;
                        media.requestBus("ytdlp_status", {}, function(raw) {
                            try {
                                var s = JSON.parse(raw);
                                if (s.error) throw s.error;
                                var ok = !!(s.ytdlp && s.ytdlp.available);
                                ytdlpChip.known = true; ytdlpChip.ok = ok;
                                ytdlpChip.label = ok ? qsTr("yt-dlp ready") : qsTr("yt-dlp not found");
                            } catch(e) {
                                // Cold start: the daemon may not be up yet; retry a few times.
                                ytdlpChip.label = qsTr("yt-dlp checking...");
                                if (ytdlpChip.tries++ < 6) ytdlpRetry.start();
                            }
                        });
                    }
                    Timer { id: ytdlpRetry; interval: 3000; onTriggered: ytdlpChip.load() }
                    Component.onCompleted: load()
                }
                Chip {
                    // ffmpegStatus arrives asynchronously and carries a tick when ffmpeg is available.
                    known: media.ffmpegStatus !== ""
                    ok: media.ffmpegStatus.indexOf("✓") >= 0
                    label: !known ? qsTr("ffmpeg checking...") : ok ? qsTr("ffmpeg ready") : qsTr("ffmpeg not found")
                    Component.onCompleted: media.onLoadFfmpegStatus()
                }
            }

            // Link
            Card {
                Row {
                    spacing: Theme.sp2
                    LabeledField {
                        label: qsTr("Video or playlist link")
                        placeholder: qsTr("https://youtube.com/watch?v=...")
                        fieldWidth: Math.max(233, parent.parent.width - 2 * Theme.btnM - 2 * Theme.sp2 - Theme.sp3)
                        onTextChanged: media.onSetMediaUrl(text)
                        onAccepted: media.onExtract()
                    }
                    Item {
                        width: extractBtn.width + Theme.sp2 + playlistBtn.width; height: Theme.controlH + Theme.sp1 + 13
                        PillButton { id: extractBtn; anchors.bottom: parent.bottom; label: media.mediaLoading ? "..." : qsTr("Extract"); onClicked: media.onExtract() }
                        PillButton { id: playlistBtn; anchors { bottom: parent.bottom; left: extractBtn.right; leftMargin: Theme.sp2 } label: qsTr("Playlist"); primary: false; onClicked: media.onPlaylistDownload() }
                    }
                }
                Text {
                    visible: media.mediaError !== ""
                    width: parent.width
                    text: media.mediaError
                    color: media.mediaError.indexOf("started") >= 0 ? Theme.success : Theme.danger
                    font.pixelSize: Theme.fontSizeSmall
                    wrapMode: Text.Wrap
                }
            }

            // Result: video info and formats
            Card {
                visible: media.mediaResult !== null
                heading: media.mediaResult ? (media.mediaResult.title || qsTr("(untitled)")) : ""
                Text {
                    width: parent.width; elide: Text.ElideRight; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                    text: {
                        if (!media.mediaResult) return "";
                        var r = media.mediaResult, parts = [];
                        if (r.uploader) parts.push(r.uploader);
                        if (r.duration_secs) parts.push(Math.round(r.duration_secs / 60) + " min");
                        if (r.extractor) parts.push(r.extractor);
                        if (r.formats) parts.push(r.formats.length + " formats");
                        return parts.join(" · ");
                    }
                }
                Row {
                    width: parent.width; spacing: 0
                    Text { anchors.verticalCenter: parent.verticalCenter; width: 30; text: "#"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    Item { width: 24; height: 1 }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: 60; text: qsTr("ID"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: 200; text: qsTr("Quality"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: 60; text: qsTr("Type"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                    Text { anchors.verticalCenter: parent.verticalCenter; width: 80; text: qsTr("Size"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                }
                Rectangle { width: parent.width; height: 1; color: Theme.border }
                ListView {
                    width: parent.width
                    height: Math.min(Theme.rowH * 8, contentHeight)
                    clip: true
                    cacheBuffer: 200
                    model: media.mediaResult ? media.mediaResult.formats : []
                    delegate: Rectangle {
                        width: ListView.view.width
                        height: Theme.rowH
                        radius: Theme.radius
                        color: media.mediaSelectedFormat === modelData.id ? Theme.accentSoft : "transparent"
                        Row {
                            anchors.fill: parent
                            Text { anchors.verticalCenter: parent.verticalCenter; width: 30; text: (index + 1); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            Rectangle {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 10; height: 10; radius: 5
                                color: media.mediaSelectedFormat === modelData.id ? Theme.accent : Theme.bg
                                border.color: media.mediaSelectedFormat === modelData.id ? Theme.accent : Theme.border; border.width: 1
                            }
                            Item { width: 14; height: 1 }
                            Text { anchors.verticalCenter: parent.verticalCenter; width: 60; text: modelData.id; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight }
                            Text { anchors.verticalCenter: parent.verticalCenter; width: 200; text: modelData.label || "-"; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight }
                            Text { anchors.verticalCenter: parent.verticalCenter; width: 60; text: modelData.ext || "-"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
                            Text {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 80
                                color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
                                text: modelData.filesize ? (modelData.filesize / (1024*1024)).toFixed(1) + " MB" : "-"
                            }
                        }
                        MouseArea { anchors.fill: parent; onClicked: media.onSetMediaSelectedFormat(modelData.id) }
                    }
                }
                PillButton {
                    label: qsTr("Download selected")
                    opacity: media.mediaSelectedFormat !== "" ? 1.0 : 0.4
                    onClicked: if (media.mediaSelectedFormat !== "") media.onMediaDownload()
                }
            }

            // Empty state
            Text {
                visible: media.mediaResult === null && !media.mediaLoading && media.mediaError === ""
                width: parent.width; horizontalAlignment: Text.AlignHCenter; topPadding: Theme.sp4; bottomPadding: Theme.sp4
                text: qsTr("Paste a video link above and press Extract to see the available qualities.")
                color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal
            }

            // Merge jobs (video + audio joined by ffmpeg)
            Card {
                visible: media.mergeJobs.length > 0
                heading: qsTr("Merge jobs")
                Repeater {
                    model: media.mergeJobs
                    Row {
                        spacing: Theme.sp2; height: Theme.rowH
                        Rectangle {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 6; height: 6; radius: 3
                            color: modelData.state === "completed" ? Theme.success : modelData.state === "failed" ? Theme.danger : Theme.warning
                        }
                        Text { anchors.verticalCenter: parent.verticalCenter; text: modelData.state || "-"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; width: 60 }
                        Text { anchors.verticalCenter: parent.verticalCenter; text: modelData.message || "-"; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight; width: 200 }
                    }
                }
                TextAction { label: qsTr("Refresh"); onClicked: media.onLoadMergeJobs() }
            }

            // Cookies for sites that need a login
            Card {
                heading: qsTr("Sign-in cookies (optional)")
                Text {
                    width: parent.width
                    text: qsTr("Some sites only serve videos to signed-in visitors. Paste the cookies.txt content exported from your browser. For privacy Ferric never reads your browser's cookie database.")
                    color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.Wrap
                }
                Rectangle {
                    width: parent.width; height: Theme.sp6; radius: Theme.radius; color: Theme.bg
                    border.width: 1; border.color: cookiesTextArea.activeFocus ? Theme.accent : Theme.border
                    clip: true
                    TextEdit {
                        id: cookiesTextArea
                        anchors { fill: parent; margins: Theme.sp2 }
                        color: Theme.text; font.pixelSize: Theme.fontSizeSmall; font.family: "monospace"
                        wrapMode: TextEdit.NoWrap
                        text: media.cookiesText
                        onTextChanged: media.onSetCookiesText(text)
                    }
                }
                Row {
                    spacing: Theme.sp2
                    PillButton { label: qsTr("Check cookies"); primary: false; onClicked: media.onParseCookies() }
                    PillButton { label: qsTr("Save to file"); primary: false; onClicked: media.onExportCookies() }
                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: media.cookiesStatus
                        color: media.cookiesStatus.indexOf("Error") >= 0 ? Theme.danger : Theme.success
                        font.pixelSize: Theme.fontSizeSmall
                    }
                }
            }
        }
    }
}
