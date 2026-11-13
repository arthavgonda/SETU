#if defined(Q_OS_LINUX) || defined(__linux__)
#include <libsecret/secret.h>
#endif

#include "secretstore.h"

#include <QCoreApplication>
#include <QLibrary>
#include <QMetaObject>
#include <QPointer>
#include <QRegularExpression>
#include <QThreadPool>

#if defined(Q_OS_MACOS)
#include <Security/Security.h>
#elif defined(Q_OS_WIN)
#include <windows.h>
#include <wincred.h>
#endif

#if defined(Q_OS_LINUX)
namespace {
using StoreFn = gboolean (*)(const SecretSchema *, const gchar *, const gchar *, const gchar *, GCancellable *, GError **, ...);
using ClearFn = gboolean (*)(const SecretSchema *, GCancellable *, GError **, ...);
using LookupFn = gchar *(*)(const SecretSchema *, GCancellable *, GError **, ...);
using FreeFn = void (*)(gchar *);
using ErrorFreeFn = void (*)(GError *);

struct Secret {
    StoreFn store = nullptr;
    ClearFn clear = nullptr;
    LookupFn lookup = nullptr;
    FreeFn free = nullptr;
    ErrorFreeFn errorFree = nullptr;
    bool ok() const { return store && clear && lookup && free && errorFree; }
};

const Secret &secretApi()
{
    static const Secret api = [] {
        Secret a;
        static QLibrary lib(QStringLiteral("secret-1"), 0);
        if (!lib.load())
            return a;
        a.store = reinterpret_cast<StoreFn>(lib.resolve("secret_password_store_sync"));
        a.clear = reinterpret_cast<ClearFn>(lib.resolve("secret_password_clear_sync"));
        a.lookup = reinterpret_cast<LookupFn>(lib.resolve("secret_password_lookup_sync"));
        a.free = reinterpret_cast<FreeFn>(lib.resolve("secret_password_free"));
        a.errorFree = reinterpret_cast<ErrorFreeFn>(lib.resolve("g_error_free"));
        return a;
    }();
    return api;
}

QString takeError(GError *err, const QString &fallback)
{
    if (!err)
        return fallback;
    const QString m = QString::fromUtf8(err->message);
    secretApi().errorFree(err);
    return m;
}
}

static const SecretSchema *schema()
{
    static const SecretSchema s = {"ai.setu.Secret",
                                   SECRET_SCHEMA_DONT_MATCH_NAME,
                                   {{"application", SECRET_SCHEMA_ATTRIBUTE_STRING}, {"ref", SECRET_SCHEMA_ATTRIBUTE_STRING}, {nullptr, SecretSchemaAttributeType(0)}},
                                   0, nullptr, nullptr, nullptr, nullptr, nullptr, nullptr, nullptr};
    return &s;
}
#endif

SecretStore::SecretStore(QObject *parent)
    : QObject(parent)
{
}

bool SecretStore::validRef(const QString &ref)
{
    static const QRegularExpression re(QStringLiteral("^[A-Za-z0-9._-]{1,64}$"));
    return re.match(ref).hasMatch();
}

QString SecretStore::storeSync(const QString &ref, const QString &secret)
{
    if (!validRef(ref))
        return QStringLiteral("Invalid key reference.");
#if defined(Q_OS_LINUX)
    const Secret &api = secretApi();
    if (!api.ok())
        return QStringLiteral("No keychain is available. Install libsecret and a Secret Service such as GNOME Keyring or KWallet.");
    GError *err = nullptr;
    const QByteArray r = ref.toUtf8();
    const QByteArray label = QStringLiteral("Setu: %1").arg(ref).toUtf8();
    QByteArray value = secret.toUtf8();
    const gboolean ok = api.store(schema(), SECRET_COLLECTION_DEFAULT, label.constData(), value.constData(), nullptr, &err,
                                  "application", "setu", "ref", r.constData(), nullptr);
    value.fill('\0');
    if (!ok)
        return takeError(err, QStringLiteral("The keychain refused the key."));
    if (err)
        api.errorFree(err);
    return {};
#elif defined(Q_OS_MACOS)
    const QByteArray acct = ref.toUtf8();
    QByteArray value = secret.toUtf8();
    CFStringRef service = CFSTR("ai.setu");
    CFStringRef account = CFStringCreateWithBytes(nullptr, reinterpret_cast<const UInt8 *>(acct.constData()), acct.size(), kCFStringEncodingUTF8, false);
    CFDataRef data = CFDataCreate(nullptr, reinterpret_cast<const UInt8 *>(value.constData()), value.size());
    const void *qk[] = {kSecClass, kSecAttrService, kSecAttrAccount};
    const void *qv[] = {kSecClassGenericPassword, service, account};
    CFDictionaryRef query = CFDictionaryCreate(nullptr, qk, qv, 3, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    SecItemDelete(query);
    const void *ak[] = {kSecClass, kSecAttrService, kSecAttrAccount, kSecValueData};
    const void *av[] = {kSecClassGenericPassword, service, account, data};
    CFDictionaryRef add = CFDictionaryCreate(nullptr, ak, av, 4, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    const OSStatus st = SecItemAdd(add, nullptr);
    CFRelease(add);
    CFRelease(query);
    CFRelease(data);
    CFRelease(account);
    value.fill('\0');
    return st == errSecSuccess ? QString() : QStringLiteral("The keychain refused the key.");
#elif defined(Q_OS_WIN)
    const std::wstring target = (QStringLiteral("ai.setu/") + ref).toStdWString();
    QByteArray value = secret.toUtf8();
    CREDENTIALW c{};
    c.Type = CRED_TYPE_GENERIC;
    c.TargetName = const_cast<LPWSTR>(target.c_str());
    c.CredentialBlobSize = DWORD(value.size());
    c.CredentialBlob = reinterpret_cast<LPBYTE>(value.data());
    c.Persist = CRED_PERSIST_LOCAL_MACHINE;
    const BOOL ok = CredWriteW(&c, 0);
    value.fill('\0');
    return ok ? QString() : QStringLiteral("Credential Manager refused the key.");
#else
    Q_UNUSED(secret)
    return QStringLiteral("No keychain is available.");
#endif
}

QString SecretStore::removeSync(const QString &ref)
{
    if (!validRef(ref))
        return QStringLiteral("Invalid key reference.");
#if defined(Q_OS_LINUX)
    const Secret &api = secretApi();
    if (!api.ok())
        return {};
    GError *err = nullptr;
    const QByteArray r = ref.toUtf8();
    api.clear(schema(), nullptr, &err, "application", "setu", "ref", r.constData(), nullptr);
    return err ? takeError(err, {}) : QString();
#elif defined(Q_OS_MACOS)
    const QByteArray acct = ref.toUtf8();
    CFStringRef account = CFStringCreateWithBytes(nullptr, reinterpret_cast<const UInt8 *>(acct.constData()), acct.size(), kCFStringEncodingUTF8, false);
    const void *qk[] = {kSecClass, kSecAttrService, kSecAttrAccount};
    const void *qv[] = {kSecClassGenericPassword, CFSTR("ai.setu"), account};
    CFDictionaryRef query = CFDictionaryCreate(nullptr, qk, qv, 3, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    SecItemDelete(query);
    CFRelease(query);
    CFRelease(account);
    return {};
#elif defined(Q_OS_WIN)
    const std::wstring target = (QStringLiteral("ai.setu/") + ref).toStdWString();
    CredDeleteW(target.c_str(), CRED_TYPE_GENERIC, 0);
    return {};
#else
    return {};
#endif
}

bool SecretStore::hasSync(const QString &ref)
{
    if (!validRef(ref))
        return false;
#if defined(Q_OS_LINUX)
    const Secret &api = secretApi();
    if (!api.ok())
        return false;
    GError *err = nullptr;
    const QByteArray r = ref.toUtf8();
    gchar *v = api.lookup(schema(), nullptr, &err, "application", "setu", "ref", r.constData(), nullptr);
    if (err)
        api.errorFree(err);
    const bool found = v != nullptr;
    if (v)
        api.free(v);
    return found;
#elif defined(Q_OS_MACOS)
    const QByteArray acct = ref.toUtf8();
    CFStringRef account = CFStringCreateWithBytes(nullptr, reinterpret_cast<const UInt8 *>(acct.constData()), acct.size(), kCFStringEncodingUTF8, false);
    const void *qk[] = {kSecClass, kSecAttrService, kSecAttrAccount};
    const void *qv[] = {kSecClassGenericPassword, CFSTR("ai.setu"), account};
    CFDictionaryRef query = CFDictionaryCreate(nullptr, qk, qv, 3, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    const bool found = SecItemCopyMatching(query, nullptr) == errSecSuccess;
    CFRelease(query);
    CFRelease(account);
    return found;
#elif defined(Q_OS_WIN)
    const std::wstring target = (QStringLiteral("ai.setu/") + ref).toStdWString();
    PCREDENTIALW c = nullptr;
    const bool found = CredReadW(target.c_str(), CRED_TYPE_GENERIC, 0, &c);
    if (c)
        CredFree(c);
    return found;
#else
    return false;
#endif
}

void SecretStore::finish(QJSValue cb, const QJSValueList &args)
{
    if (cb.isCallable())
        cb.call(args);
}

void SecretStore::store(const QString &ref, const QString &secret, const QJSValue &callback)
{
    QPointer<SecretStore> me(this);
    QJSValue cb = callback;
    QThreadPool::globalInstance()->start([me, ref, secret, cb]() mutable {
        const QString err = storeSync(ref, secret);
        QMetaObject::invokeMethod(qApp, [me, cb, err]() mutable {
            if (me)
                me->finish(cb, {QJSValue(err.isEmpty()), QJSValue(err)});
        });
    });
}

void SecretStore::remove(const QString &ref, const QJSValue &callback)
{
    QPointer<SecretStore> me(this);
    QJSValue cb = callback;
    QThreadPool::globalInstance()->start([me, ref, cb]() mutable {
        const QString err = removeSync(ref);
        QMetaObject::invokeMethod(qApp, [me, cb, err]() mutable {
            if (me)
                me->finish(cb, {QJSValue(err.isEmpty()), QJSValue(err)});
        });
    });
}

void SecretStore::has(const QString &ref, const QJSValue &callback)
{
    QPointer<SecretStore> me(this);
    QJSValue cb = callback;
    QThreadPool::globalInstance()->start([me, ref, cb]() mutable {
        const bool found = hasSync(ref);
        QMetaObject::invokeMethod(qApp, [me, cb, found]() mutable {
            if (me)
                me->finish(cb, {QJSValue(found)});
        });
    });
}
