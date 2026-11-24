// Oficina's window: the workbench, its panes docked by KDDockWidgets. The project pane first; the
// editors, the inspector and the rest arrive with their clauses (docs/oficina-plan.md).
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
                id: projectPane
                objectName: "projectPane"
                anchors.fill: parent
            }
        }

        Component.onCompleted: addDockWidget(projectDock, KDDW.KDDockWidgets.Location_OnLeft, null, Qt.size(320, 800))
    }
}
