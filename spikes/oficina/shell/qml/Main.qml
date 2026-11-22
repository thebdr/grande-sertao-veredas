import QtQuick
import QtQuick.Controls
import com.kdab.dockwidgets 2.0 as KDDW

ApplicationWindow {
    id: window
    objectName: "oficina"
    visible: true
    width: 1280
    height: 800
    title: "Oficina"
    color: "#141416"

    KDDW.DockingArea {
        id: area
        objectName: "area"
        anchors.fill: parent
        uniqueName: "workbench"

        KDDW.DockWidget {
            id: projectDock
            uniqueName: "project"
            title: qsTr("Project")
            ProjectPane {
                anchors.fill: parent
            }
        }

        KDDW.DockWidget {
            id: editorDock
            uniqueName: "editor"
            title: qsTr("Editor")
            Rectangle {
                anchors.fill: parent
                color: "#1B1B1E"
                Button {
                    objectName: "hello"
                    property int clicks: 0
                    text: "clicked " + clicks
                    onClicked: clicks += 1
                }
            }
        }

        Component.onCompleted: {
            addDockWidget(projectDock, KDDW.KDDockWidgets.Location_OnLeft, null, Qt.size(320, 800));
            addDockWidget(editorDock, KDDW.KDDockWidgets.Location_OnRight);
        }
    }
}
