#include "oficina-spike-shell/cpp/shell.h"

#include <memory>
#include <string>
#include <vector>

#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QtWebEngineQuick/QtWebEngineQuick>

#include <kddockwidgets/Config.h>
#include <kddockwidgets/qtquick/Platform.h>

#include <Spix/AnyRpcServer.h>
#include <Spix/QtQmlBot.h>

::std::int32_t oficina_run(::rust::Vec<::rust::String> args)
{
    // WebEngine shares its OpenGL context with Qt Quick: it is set up before the application.
    QtWebEngineQuick::initialize();

    std::vector<std::string> owned;
    for (const auto &a : args)
        owned.emplace_back(a.data(), a.size());
    std::vector<char *> argv;
    for (auto &a : owned)
        argv.push_back(a.data());
    argv.push_back(nullptr);
    int argc = static_cast<int>(owned.size());

    QGuiApplication app(argc, argv.data());
    QGuiApplication::setApplicationName(QStringLiteral("Oficina"));

    KDDockWidgets::initFrontend(KDDockWidgets::FrontendType::QtQuick);
    QQmlApplicationEngine engine;
    KDDockWidgets::QtQuick::Platform::instance()->setQmlEngine(&engine);

    // The test server, only when a test asks for it by naming its port.
    std::unique_ptr<spix::AnyRpcServer> server;
    std::unique_ptr<spix::QtQmlBot> bot;
    const int port = qEnvironmentVariableIntValue("OFICINA_SPIX_PORT");
    if (port > 0) {
        server = std::make_unique<spix::AnyRpcServer>(port);
        bot = std::make_unique<spix::QtQmlBot>();
        bot->runTestServer(*server);
    }

    engine.loadFromModule("Oficina", "Main");
    if (engine.rootObjects().isEmpty())
        return 1;
    return app.exec();
}
