import QtQuick
import QtQuick.Window
import QtQuick.Controls as QQC
import com.ferric.app 1.0

// IDM's zip Preview: the archive's file list, read from the end of the remote file.
Window {
    id: win
    property string downloadId: ""
    property var requestBus
    property var entries: []
    property string status: qsTr("Reading the archive directory\u2026")

    width: 460
    height: 380
    minimumWidth: 360
    minimumHeight: 240
    color: Theme.bg
    title: qsTr("Zip preview")
    flags: Qt.Dialog | Qt.WindowTitleHint | Qt.WindowCloseButtonHint | Qt.WindowStaysOnTopHint

    function fmtSize(n) {
        if (n < 1024) return n + " B";
        if (n < 1048576) return (n / 1024).toFixed(1) + " KB";
        return (n / 1048576).toFixed(2) + " MB";
    }

    Component.onCompleted: requestBus("get_zip_preview", { id: downloadId }, function(raw) {
        var v;
        try { v = JSON.parse(raw); } catch (e) { win.status = raw || qsTr("Preview failed"); return; }
        if (v instanceof Array) { win.entries = v; win.status = v.length === 0 ? qsTr("The archive is empty") : ""; }
        else { win.status = (v && v.error) ? v.error : qsTr("Preview failed"); }
    })

    Text {
        anchors { left: parent.left; right: parent.right; top: parent.top; margins: Theme.sp3 }
        visible: win.status !== ""
        text: win.status
        color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal; wrapMode: Text.Wrap
    }

    ListView {
        anchors { fill: parent; margins: Theme.sp2; bottomMargin: Theme.sp6 }
        visible: win.entries.length > 0
        clip: true
        model: win.entries.length
        QQC.ScrollBar.vertical: QQC.ScrollBar {}
        header: Row {
            height: Theme.rowH
            Text { width: 300; text: qsTr("Name"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
            Text { text: qsTr("Size"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
        }
        delegate: Row {
            readonly property var e: win.entries[index]
            height: Theme.rowH
            Text { width: 300; text: e ? e.name : ""; textFormat: Text.PlainText; color: Theme.text; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideMiddle }
            Text { text: e ? win.fmtSize(e.size) : ""; color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
        }
    }

    Rectangle {
        anchors { horizontalCenter: parent.horizontalCenter; bottom: parent.bottom; bottomMargin: Theme.sp2 }
        width: 90; height: Theme.controlH; radius: Theme.radius; color: Theme.accent
        Text { anchors.centerIn: parent; text: qsTr("Close"); color: "white"; font.pixelSize: Theme.fontSizeNormal }
        MouseArea { anchors.fill: parent; onClicked: win.close() }
    }
}
