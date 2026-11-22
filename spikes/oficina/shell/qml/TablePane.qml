// A million rows from a Rust model (the spike's point 3): only the cells drawn are asked for.
import QtQuick
import QtQuick.Controls
import Oficina

Rectangle {
    id: pane
    objectName: "tablePane"
    color: "#1B1B1E"

    // How many rows the Open button shows: `--table-rows=N` on the command line, a million else.
    readonly property int wanted: {
        const args = Qt.application.arguments;
        for (let i = 1; i < args.length; i++)
            if (args[i].startsWith("--table-rows="))
                return parseInt(args[i].substring(13));
        return 1000000;
    }

    // Bring a cell into view, its row at the bottom, as a "go to cell" command does.
    function jumpTo(row, column) {
        table.positionViewAtCell(Qt.point(column, row), TableView.AlignBottom | TableView.AlignRight);
    }

    // Scroll the table steadily for `ms` at `rowsPerSecond` and time each frame meanwhile (point 3's
    // frame times, taken with point 6): `frames` then reads {frames, p50, p95, max}, the intervals
    // in milliseconds between the window's frames as its GUI thread advances them (`afterAnimating`,
    // the thread that makes the rows' cells).
    property string frames: ""
    property var ticks: []

    function scrollFor(ms, rowsPerSecond) {
        frames = "";
        ticks = [];
        scroll.from = table.contentY;
        scroll.to = table.contentY + rowsPerSecond * (ms / 1000) * (20 + table.rowSpacing);
        scroll.duration = ms;
        scroll.start();
    }

    NumberAnimation {
        id: scroll
        target: table
        property: "contentY"
        easing.type: Easing.Linear
        onStopped: {
            const gaps = [];
            for (let i = 1; i < pane.ticks.length; i++)
                gaps.push(pane.ticks[i] - pane.ticks[i - 1]);
            gaps.sort((a, b) => a - b);
            const at = (q) => gaps.length ? gaps[Math.min(gaps.length - 1, Math.floor(q * gaps.length))] : null;
            pane.frames = JSON.stringify({ frames: gaps.length, p50: at(0.5), p95: at(0.95), max: gaps.length ? gaps[gaps.length - 1] : null });
        }
    }

    Connections {
        target: pane.Window.window
        enabled: scroll.running
        function onAfterAnimating() {
            pane.ticks.push(Date.now());
        }
    }

    TableSource {
        id: source
    }

    Row {
        id: bar
        spacing: 8
        Button {
            objectName: "openTable"
            text: qsTr("Open %1 rows").arg(pane.wanted)
            onClicked: source.showRows(pane.wanted)
        }
        Text {
            objectName: "tableRows"
            text: source.rows
            color: "#9A9AA3"
            anchors.verticalCenter: parent.verticalCenter
        }
    }

    HorizontalHeaderView {
        id: header
        anchors { top: bar.bottom; left: table.left; right: table.right }
        syncView: table
        clip: true
    }

    TableView {
        id: table
        objectName: "table"
        anchors { top: header.bottom; left: parent.left; right: parent.right; bottom: parent.bottom }
        clip: true
        reuseItems: false
        columnSpacing: 1
        rowSpacing: 1
        model: source
        delegate: Rectangle {
            required property int row
            required property int column
            required property string display
            objectName: "cell_" + row + "_" + column
            implicitWidth: 110
            implicitHeight: 20
            color: "#191A1D"
            Text {
                anchors { fill: parent; leftMargin: 4 }
                text: parent.display
                textFormat: Text.PlainText
                color: "#D4D4DA"
                font.family: "monospace"
                verticalAlignment: Text.AlignVCenter
            }
        }
    }
}
