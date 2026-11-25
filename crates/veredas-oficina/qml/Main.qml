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

        // The docks as they stand, for a test to read (Spix reaches items, not the docks
        // themselves): each dock's name, open, floating, and whether its pane is drawn in this
        // window, set by `describeDocks`.
        property string docks: ""

        function describeDocks() {
            docks = JSON.stringify([[projectDock, projectPane]].map(([dock, pane]) => [dock.uniqueName, dock.isOpen, dock.isFloating, pane.Window.window === window]));
        }

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
