#include "veredas-oficina/cpp/shell.h"

#include <memory>
#include <string>
#include <vector>

#include <QGuiApplication>
#include <QQmlApplicationEngine>

#include <kddockwidgets/Config.h>
#include <kddockwidgets/qtquick/Platform.h>

#ifdef OFICINA_DRIVER
#include <Spix/AnyRpcServer.h>
#include <Spix/QtQmlBot.h>
#endif

::std::int32_t oficina_run(::rust::Vec<::rust::String> args)
{
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

#ifdef OFICINA_DRIVER
    // The tests' driver (the `driver` feature, never in a build that ships): Spix's server, started
    // only when a test names its port.
    std::unique_ptr<spix::AnyRpcServer> server;
    std::unique_ptr<spix::QtQmlBot> bot;
    const int port = qEnvironmentVariableIntValue("OFICINA_SPIX_PORT");
    if (port > 0) {
        server = std::make_unique<spix::AnyRpcServer>(port);
        bot = std::make_unique<spix::QtQmlBot>();
        bot->runTestServer(*server);
    }
#endif

    engine.loadFromModule("Oficina", "Main");
    if (engine.rootObjects().isEmpty())
        return 1;
    return app.exec();
}
