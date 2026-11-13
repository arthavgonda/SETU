#include "lottie.h"

#include <QFile>
#include <QHash>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLinearGradient>
#include <QLoggingCategory>
#include <QMutex>
#include <QPainter>
#include <QPainterPath>
#include <QQmlFile>
#include <QQuickWindow>
#include <QRadialGradient>
#include <QtMath>
#include <cmath>
#include <vector>

Q_LOGGING_CATEGORY(lcLottie, "setu.lottie")

namespace lottie {

using Vec = std::vector<double>;

struct Ease
{
    Vec x, y;
};

struct Key
{
    double t = 0;
    Vec s, e;
    bool hasE = false;
    bool hold = false;
    Ease i, o;
    Vec ti, to;
};

struct Prop
{
    Vec value;
    std::vector<Key> keys;
    bool present = false;
    bool isPath = false;
    Vec at(double t) const;
    double scalar(double t, double fallback) const
    {
        const Vec v = at(t);
        return v.empty() ? fallback : v[0];
    }
};

struct Transform
{
    Prop a, p, px, py, s, r, o, sk, sa;
    bool split = false;
    QTransform matrix(double t) const;
    double opacity(double t) const { return o.present ? o.scalar(t, 100) / 100.0 : 1.0; }
};

enum class Kind { Group, Path, Ellipse, Rect, Star, Fill, Stroke, GradientFill, GradientStroke, Trim, Other };

struct Item
{
    Kind kind = Kind::Other;
    bool hidden = false;
    std::vector<Item> items;
    Transform tr;
    Prop shape, pos, size, round, rotation, outer, inner, points;
    int starType = 1;
    int direction = 1;
    Prop color, opacity, width;
    int cap = 2;
    int join = 2;
    double miter = 4;
    int rule = 1;
    std::vector<std::pair<QString, Prop>> dashes;
    Prop start, end, offset;
    Prop gStart, gEnd, gStops;
    int gType = 1;
    int gCount = 0;
};

struct Mask
{
    QString mode;
    bool inverted = false;
    Prop path;
};

struct Layer
{
    int ind = -1;
    int parent = -1;
    int type = -1;
    double ip = 0, op = 0, st = 0, sr = 1;
    bool hidden = false;
    bool matteSource = false;
    int matte = 0;
    Transform ks;
    std::vector<Item> shapes;
    std::vector<Mask> masks;
    QString ref;
    double w = 0, h = 0;
    Prop remap;
    QColor solid;
    double sw = 0, sh = 0;
};

struct Asset
{
    std::vector<Layer> layers;
    QImage image;
};

struct Composition
{
    double w = 0, h = 0, fr = 30, ip = 0, op = 0;
    std::vector<Layer> layers;
    QHash<QString, Asset> assets;
};

static Vec flattenPath(const QJsonObject &o)
{
    const QJsonArray v = o.value(QLatin1String("v")).toArray();
    const QJsonArray in = o.value(QLatin1String("i")).toArray();
    const QJsonArray out = o.value(QLatin1String("o")).toArray();
    const int n = v.size();
    Vec r;
    r.reserve(2 + n * 6);
    r.push_back(o.value(QLatin1String("c")).toBool() ? 1 : 0);
    r.push_back(n);
    for (const QJsonArray *arr : { &v, &in, &out })
        for (int k = 0; k < n; ++k) {
            const QJsonArray pt = arr->at(k).toArray();
            r.push_back(pt.at(0).toDouble());
            r.push_back(pt.at(1).toDouble());
        }
    return r;
}

static Vec toVec(const QJsonValue &v)
{
    if (v.isDouble())
        return { v.toDouble() };
    if (v.isBool())
        return { v.toBool() ? 1.0 : 0.0 };
    if (v.isObject())
        return v.toObject().contains(QLatin1String("v")) ? flattenPath(v.toObject()) : Vec();
    if (v.isArray()) {
        const QJsonArray a = v.toArray();
        if (!a.isEmpty() && a.first().isObject())
            return flattenPath(a.first().toObject());
        Vec r;
        r.reserve(a.size());
        for (const auto &x : a)
            r.push_back(x.toDouble());
        return r;
    }
    return {};
}

static Ease toEase(const QJsonValue &v)
{
    const QJsonObject o = v.toObject();
    return { toVec(o.value(QLatin1String("x"))), toVec(o.value(QLatin1String("y"))) };
}

static Prop toProp(const QJsonValue &v)
{
    Prop p;
    if (!v.isObject())
        return p;
    p.present = true;
    const QJsonValue k = v.toObject().value(QLatin1String("k"));
    const bool keyed = k.isArray() && !k.toArray().isEmpty() && k.toArray().first().isObject() && k.toArray().first().toObject().contains(QLatin1String("t"));
    if (!keyed) {
        p.value = toVec(k);
        return p;
    }
    for (const auto &e : k.toArray()) {
        const QJsonObject o = e.toObject();
        Key key;
        key.t = o.value(QLatin1String("t")).toDouble();
        key.s = toVec(o.value(QLatin1String("s")));
        if (o.contains(QLatin1String("e"))) {
            key.e = toVec(o.value(QLatin1String("e")));
            key.hasE = true;
        }
        key.hold = o.value(QLatin1String("h")).toInt() == 1;
        key.i = toEase(o.value(QLatin1String("i")));
        key.o = toEase(o.value(QLatin1String("o")));
        key.ti = toVec(o.value(QLatin1String("ti")));
        key.to = toVec(o.value(QLatin1String("to")));
        p.keys.push_back(std::move(key));
    }
    return p;
}

static double easeAt(const Ease &out, const Ease &in, size_t dim, double p)
{
    auto pick = [dim](const Vec &v, double fallback) {
        if (v.empty())
            return fallback;
        return dim < v.size() ? v[dim] : v[0];
    };
    const double x1 = qBound(0.0, pick(out.x, 0), 1.0), y1 = pick(out.y, 0);
    const double x2 = qBound(0.0, pick(in.x, 1), 1.0), y2 = pick(in.y, 1);
    if (out.x.empty() && in.x.empty())
        return p;
    auto bx = [&](double u) { return 3 * (1 - u) * (1 - u) * u * x1 + 3 * (1 - u) * u * u * x2 + u * u * u; };
    auto by = [&](double u) { return 3 * (1 - u) * (1 - u) * u * y1 + 3 * (1 - u) * u * u * y2 + u * u * u; };
    auto dx = [&](double u) { return 3 * (1 - u) * (1 - u) * x1 + 6 * (1 - u) * u * (x2 - x1) + 3 * u * u * (1 - x2); };
    double u = p;
    for (int n = 0; n < 8; ++n) {
        const double err = bx(u) - p;
        if (std::abs(err) < 1e-6)
            return by(u);
        const double d = dx(u);
        if (std::abs(d) < 1e-6)
            break;
        u -= err / d;
    }
    if (u < 0 || u > 1 || std::abs(bx(u) - p) > 1e-4) {
        double lo = 0, hi = 1;
        u = p;
        for (int n = 0; n < 40; ++n) {
            if (bx(u) < p)
                lo = u;
            else
                hi = u;
            u = (lo + hi) / 2;
        }
    }
    return by(u);
}

static bool nonZero(const Vec &v)
{
    for (double x : v)
        if (std::abs(x) > 1e-9)
            return true;
    return false;
}

Vec Prop::at(double t) const
{
    if (keys.empty())
        return value;
    const Key &first = keys.front();
    if (t <= first.t)
        return first.s.empty() ? first.e : first.s;
    for (size_t n = 0; n + 1 < keys.size(); ++n) {
        const Key &a = keys[n];
        const Key &b = keys[n + 1];
        if (t >= b.t)
            continue;
        const Vec &end = a.hasE ? a.e : b.s;
        if (a.hold || end.size() != a.s.size() || a.s.empty())
            return a.s;
        const double span = b.t - a.t;
        const double p = span > 0 ? (t - a.t) / span : 1;
        Vec r(a.s.size());
        const bool spatial = a.s.size() >= 2 && (nonZero(a.to) || nonZero(a.ti)) && a.to.size() >= 2 && a.ti.size() >= 2;
        if (spatial) {
            const double q = easeAt(a.o, a.i, 0, p);
            const QPointF p0(a.s[0], a.s[1]), p3(end[0], end[1]);
            const QPointF p1 = p0 + QPointF(a.to[0], a.to[1]), p2 = p3 + QPointF(a.ti[0], a.ti[1]);
            auto pt = [&](double u) {
                const double m = 1 - u;
                return m * m * m * p0 + 3 * m * m * u * p1 + 3 * m * u * u * p2 + u * u * u * p3;
            };
            constexpr int samples = 24;
            double len[samples + 1];
            len[0] = 0;
            QPointF prev = p0;
            for (int k = 1; k <= samples; ++k) {
                const QPointF c = pt(double(k) / samples);
                len[k] = len[k - 1] + std::hypot(c.x() - prev.x(), c.y() - prev.y());
                prev = c;
            }
            const double want = q * len[samples];
            double u = q;
            if (len[samples] > 0) {
                int k = 1;
                while (k < samples && len[k] < want)
                    ++k;
                const double seg = len[k] - len[k - 1];
                u = (k - 1 + (seg > 0 ? (want - len[k - 1]) / seg : 0)) / samples;
            }
            const QPointF c = pt(u);
            r[0] = c.x();
            r[1] = c.y();
            for (size_t d = 2; d < r.size(); ++d)
                r[d] = a.s[d] + (end[d] - a.s[d]) * q;
            return r;
        }
        const bool path = isPath;
        for (size_t d = 0; d < r.size(); ++d) {
            const double q = path ? easeAt(a.o, a.i, 0, p) : easeAt(a.o, a.i, d, p);
            r[d] = a.s[d] + (end[d] - a.s[d]) * q;
        }
        if (path) {
            r[0] = a.s[0];
            r[1] = a.s[1];
        }
        return r;
    }
    const Key &last = keys.back();
    if (!last.s.empty())
        return last.s;
    const Key &prev = keys[keys.size() >= 2 ? keys.size() - 2 : 0];
    return prev.hasE ? prev.e : prev.s;
}

QTransform Transform::matrix(double t) const
{
    const Vec av = a.at(t);
    double x = 0, y = 0;
    if (split) {
        x = px.scalar(t, 0);
        y = py.scalar(t, 0);
    } else {
        const Vec pv = p.at(t);
        x = pv.size() > 0 ? pv[0] : 0;
        y = pv.size() > 1 ? pv[1] : 0;
    }
    const Vec sv = s.at(t);
    const double sx = s.present && !sv.empty() ? sv[0] / 100.0 : 1.0;
    const double sy = s.present && sv.size() > 1 ? sv[1] / 100.0 : sx;
    const double rot = r.scalar(t, 0);
    const double skew = sk.scalar(t, 0);
    QTransform m;
    m.translate(x, y);
    m.rotate(rot);
    if (std::abs(skew) > 1e-6) {
        const double axis = sa.scalar(t, 0);
        m.rotate(axis);
        m.shear(std::tan(qDegreesToRadians(-skew)), 0);
        m.rotate(-axis);
    }
    m.scale(sx, sy);
    m.translate(av.size() > 0 ? -av[0] : 0, av.size() > 1 ? -av[1] : 0);
    return m;
}

static Transform toTransform(const QJsonObject &o)
{
    Transform t;
    t.a = toProp(o.value(QLatin1String("a")));
    const QJsonObject p = o.value(QLatin1String("p")).toObject();
    if (p.value(QLatin1String("s")).toBool()) {
        t.split = true;
        t.px = toProp(p.value(QLatin1String("x")));
        t.py = toProp(p.value(QLatin1String("y")));
    } else {
        t.p = toProp(p);
    }
    t.s = toProp(o.value(QLatin1String("s")));
    t.r = toProp(o.contains(QLatin1String("r")) ? o.value(QLatin1String("r")) : o.value(QLatin1String("rz")));
    t.o = toProp(o.value(QLatin1String("o")));
    t.sk = toProp(o.value(QLatin1String("sk")));
    t.sa = toProp(o.value(QLatin1String("sa")));
    return t;
}

static std::vector<Item> toItems(const QJsonArray &arr);

static Item toItem(const QJsonObject &o)
{
    Item it;
    const QString ty = o.value(QLatin1String("ty")).toString();
    it.hidden = o.value(QLatin1String("hd")).toBool();
    it.direction = o.value(QLatin1String("d")).toInt(1);
    auto prop = [&o](const char *k) { return toProp(o.value(QLatin1String(k))); };
    if (ty == QLatin1String("gr")) {
        it.kind = Kind::Group;
        for (const auto &v : o.value(QLatin1String("it")).toArray())
            if (v.toObject().value(QLatin1String("ty")).toString() == QLatin1String("tr"))
                it.tr = toTransform(v.toObject());
        it.items = toItems(o.value(QLatin1String("it")).toArray());
    } else if (ty == QLatin1String("sh")) {
        it.kind = Kind::Path;
        it.shape = prop("ks");
        it.shape.isPath = true;
    } else if (ty == QLatin1String("el")) {
        it.kind = Kind::Ellipse;
        it.pos = prop("p");
        it.size = prop("s");
    } else if (ty == QLatin1String("rc")) {
        it.kind = Kind::Rect;
        it.pos = prop("p");
        it.size = prop("s");
        it.round = prop("r");
    } else if (ty == QLatin1String("sr")) {
        it.kind = Kind::Star;
        it.pos = prop("p");
        it.outer = prop("or");
        it.inner = prop("ir");
        it.points = prop("pt");
        it.rotation = prop("r");
        it.starType = o.value(QLatin1String("sy")).toInt(1);
    } else if (ty == QLatin1String("fl") || ty == QLatin1String("gf")) {
        it.kind = ty == QLatin1String("fl") ? Kind::Fill : Kind::GradientFill;
        it.color = prop("c");
        it.opacity = prop("o");
        it.rule = o.value(QLatin1String("r")).toInt(1);
    } else if (ty == QLatin1String("st") || ty == QLatin1String("gs")) {
        it.kind = ty == QLatin1String("st") ? Kind::Stroke : Kind::GradientStroke;
        it.color = prop("c");
        it.opacity = prop("o");
        it.width = prop("w");
        it.cap = o.value(QLatin1String("lc")).toInt(2);
        it.join = o.value(QLatin1String("lj")).toInt(2);
        it.miter = o.value(QLatin1String("ml")).toDouble(4);
        for (const auto &d : o.value(QLatin1String("d")).toArray()) {
            const QJsonObject dd = d.toObject();
            it.dashes.emplace_back(dd.value(QLatin1String("n")).toString(), toProp(dd.value(QLatin1String("v"))));
        }
    } else if (ty == QLatin1String("tm")) {
        it.kind = Kind::Trim;
        it.start = prop("s");
        it.end = prop("e");
        it.offset = prop("o");
    } else {
        it.kind = Kind::Other;
    }
    if (it.kind == Kind::GradientFill || it.kind == Kind::GradientStroke) {
        it.gStart = prop("s");
        it.gEnd = prop("e");
        it.gType = o.value(QLatin1String("t")).toInt(1);
        const QJsonObject g = o.value(QLatin1String("g")).toObject();
        it.gCount = g.value(QLatin1String("p")).toInt();
        it.gStops = toProp(g.value(QLatin1String("k")));
    }
    return it;
}

static std::vector<Item> toItems(const QJsonArray &arr)
{
    std::vector<Item> out;
    out.reserve(arr.size());
    for (const auto &v : arr) {
        Item it = toItem(v.toObject());
        if (it.kind != Kind::Other)
            out.push_back(std::move(it));
    }
    return out;
}

static std::vector<Layer> toLayers(const QJsonArray &arr)
{
    std::vector<Layer> out;
    out.reserve(arr.size());
    for (const auto &v : arr) {
        const QJsonObject o = v.toObject();
        Layer l;
        l.ind = o.value(QLatin1String("ind")).toInt(-1);
        l.parent = o.contains(QLatin1String("parent")) ? o.value(QLatin1String("parent")).toInt(-1) : -1;
        l.type = o.value(QLatin1String("ty")).toInt(-1);
        l.ip = o.value(QLatin1String("ip")).toDouble();
        l.op = o.value(QLatin1String("op")).toDouble();
        l.st = o.value(QLatin1String("st")).toDouble();
        l.sr = o.value(QLatin1String("sr")).toDouble(1);
        if (l.sr == 0)
            l.sr = 1;
        l.hidden = o.value(QLatin1String("hd")).toBool();
        l.matteSource = o.value(QLatin1String("td")).toInt() != 0;
        l.matte = o.value(QLatin1String("tt")).toInt();
        l.ks = toTransform(o.value(QLatin1String("ks")).toObject());
        l.shapes = toItems(o.value(QLatin1String("shapes")).toArray());
        if (o.value(QLatin1String("hasMask")).toBool())
            for (const auto &m : o.value(QLatin1String("masksProperties")).toArray()) {
                const QJsonObject mo = m.toObject();
                Mask mask { mo.value(QLatin1String("mode")).toString(QStringLiteral("a")), mo.value(QLatin1String("inv")).toBool(), toProp(mo.value(QLatin1String("pt"))) };
                mask.path.isPath = true;
                l.masks.push_back(std::move(mask));
            }
        l.ref = o.value(QLatin1String("refId")).toString();
        l.w = o.value(QLatin1String("w")).toDouble();
        l.h = o.value(QLatin1String("h")).toDouble();
        l.remap = toProp(o.value(QLatin1String("tm")));
        l.solid = QColor(o.value(QLatin1String("sc")).toString());
        l.sw = o.value(QLatin1String("sw")).toDouble();
        l.sh = o.value(QLatin1String("sh")).toDouble();
        out.push_back(std::move(l));
    }
    return out;
}

static std::shared_ptr<Composition> parse(const QByteArray &data, QString *error)
{
    QJsonParseError pe;
    const QJsonDocument doc = QJsonDocument::fromJson(data, &pe);
    if (!doc.isObject()) {
        *error = pe.errorString();
        return nullptr;
    }
    const QJsonObject o = doc.object();
    auto c = std::make_shared<Composition>();
    c->w = o.value(QLatin1String("w")).toDouble();
    c->h = o.value(QLatin1String("h")).toDouble();
    c->fr = o.value(QLatin1String("fr")).toDouble(30);
    c->ip = o.value(QLatin1String("ip")).toDouble();
    c->op = o.value(QLatin1String("op")).toDouble();
    if (c->w <= 0 || c->h <= 0 || c->fr <= 0 || c->op <= c->ip) {
        *error = QStringLiteral("not a Lottie animation");
        return nullptr;
    }
    c->layers = toLayers(o.value(QLatin1String("layers")).toArray());
    for (const auto &v : o.value(QLatin1String("assets")).toArray()) {
        const QJsonObject a = v.toObject();
        Asset asset;
        if (a.contains(QLatin1String("layers"))) {
            asset.layers = toLayers(a.value(QLatin1String("layers")).toArray());
        } else {
            const QString p = a.value(QLatin1String("p")).toString();
            const int comma = p.indexOf(QLatin1Char(','));
            if (p.startsWith(QLatin1String("data:image/")) && comma > 0)
                asset.image = QImage::fromData(QByteArray::fromBase64(p.mid(comma + 1).toLatin1()));
        }
        c->assets.insert(a.value(QLatin1String("id")).toString(), std::move(asset));
    }
    return c;
}

static QPainterPath pathFrom(const Vec &v)
{
    QPainterPath path;
    if (v.size() < 2)
        return path;
    const int n = int(v[1]);
    if (n < 1 || v.size() < size_t(2 + n * 6))
        return path;
    auto pt = [&](int block, int k) { return QPointF(v[2 + block * n * 2 + k * 2], v[2 + block * n * 2 + k * 2 + 1]); };
    path.moveTo(pt(0, 0));
    for (int k = 1; k < n; ++k)
        path.cubicTo(pt(0, k - 1) + pt(2, k - 1), pt(0, k) + pt(1, k), pt(0, k));
    if (v[0] > 0.5) {
        path.cubicTo(pt(0, n - 1) + pt(2, n - 1), pt(0, 0) + pt(1, 0), pt(0, 0));
        path.closeSubpath();
    }
    return path;
}

static QPointF point(const Vec &v)
{
    return QPointF(v.size() > 0 ? v[0] : 0, v.size() > 1 ? v[1] : 0);
}

static QPainterPath ellipse(const Item &it, double t)
{
    const QPointF c = point(it.pos.at(t));
    const QPointF s = point(it.size.at(t));
    const double rx = s.x() / 2, ry = s.y() / 2;
    constexpr double k = 0.5519150244935105;
    const QPointF top(c.x(), c.y() - ry), right(c.x() + rx, c.y()), bottom(c.x(), c.y() + ry), left(c.x() - rx, c.y());
    QPainterPath p;
    p.moveTo(top);
    if (it.direction == 3) {
        p.cubicTo(top + QPointF(-k * rx, 0), left + QPointF(0, -k * ry), left);
        p.cubicTo(left + QPointF(0, k * ry), bottom + QPointF(-k * rx, 0), bottom);
        p.cubicTo(bottom + QPointF(k * rx, 0), right + QPointF(0, k * ry), right);
        p.cubicTo(right + QPointF(0, -k * ry), top + QPointF(k * rx, 0), top);
    } else {
        p.cubicTo(top + QPointF(k * rx, 0), right + QPointF(0, -k * ry), right);
        p.cubicTo(right + QPointF(0, k * ry), bottom + QPointF(k * rx, 0), bottom);
        p.cubicTo(bottom + QPointF(-k * rx, 0), left + QPointF(0, k * ry), left);
        p.cubicTo(left + QPointF(0, -k * ry), top + QPointF(-k * rx, 0), top);
    }
    p.closeSubpath();
    return p;
}

static QPainterPath rect(const Item &it, double t)
{
    const QPointF c = point(it.pos.at(t));
    const QPointF s = point(it.size.at(t));
    const double hw = s.x() / 2, hh = s.y() / 2;
    const double r = qMin(it.round.scalar(t, 0), qMin(hw, hh));
    QPainterPath p;
    if (r > 0) {
        p.addRoundedRect(QRectF(c.x() - hw, c.y() - hh, s.x(), s.y()), r, r);
        return p;
    }
    const QPointF tr(c.x() + hw, c.y() - hh), br(c.x() + hw, c.y() + hh), bl(c.x() - hw, c.y() + hh), tl(c.x() - hw, c.y() - hh);
    p.moveTo(tr);
    if (it.direction == 3) {
        p.lineTo(tl);
        p.lineTo(bl);
        p.lineTo(br);
    } else {
        p.lineTo(br);
        p.lineTo(bl);
        p.lineTo(tl);
    }
    p.closeSubpath();
    return p;
}

static QPainterPath star(const Item &it, double t)
{
    const QPointF c = point(it.pos.at(t));
    const int n = qBound(3, int(std::round(it.points.scalar(t, 5))), 100);
    const double outer = it.outer.scalar(t, 0), inner = it.inner.scalar(t, 0);
    const double start = qDegreesToRadians(it.rotation.scalar(t, 0) - 90);
    const bool isStar = it.starType == 1;
    const int count = isStar ? n * 2 : n;
    QPainterPath p;
    for (int k = 0; k < count; ++k) {
        const double radius = isStar && (k % 2) ? inner : outer;
        const double a = start + k * 2 * M_PI / count * (it.direction == 3 ? -1 : 1);
        const QPointF q(c.x() + radius * std::cos(a), c.y() + radius * std::sin(a));
        if (k == 0)
            p.moveTo(q);
        else
            p.lineTo(q);
    }
    p.closeSubpath();
    return p;
}

struct Trim
{
    double s, e, o;
};

static QPainterPath trimPath(const QPainterPath &path, const Trim &tr)
{
    double s = qBound(0.0, tr.s, 1.0), e = qBound(0.0, tr.e, 1.0);
    if (s > e)
        std::swap(s, e);
    if (e - s >= 1 - 1e-6)
        return path;
    if (e - s <= 1e-6)
        return {};
    s += tr.o;
    e += tr.o;
    const double shift = std::floor(s);
    s -= shift;
    e -= shift;
    const QList<QPolygonF> polys = path.toSubpathPolygons();
    double total = 0;
    for (const QPolygonF &poly : polys)
        for (int k = 1; k < poly.size(); ++k)
            total += QLineF(poly[k - 1], poly[k]).length();
    if (total <= 0)
        return {};
    QPainterPath out;
    auto take = [&](double from, double to) {
        double walked = 0;
        for (const QPolygonF &poly : polys) {
            bool open = false;
            for (int k = 1; k < poly.size(); ++k) {
                const QLineF seg(poly[k - 1], poly[k]);
                const double len = seg.length();
                const double a = walked, b = walked + len;
                walked = b;
                if (b < from || a > to || len <= 0) {
                    open = false;
                    continue;
                }
                const QPointF p0 = seg.pointAt(qMax(0.0, (from - a) / len));
                const QPointF p1 = seg.pointAt(qMin(1.0, (to - a) / len));
                if (!open) {
                    out.moveTo(p0);
                    open = true;
                }
                out.lineTo(p1);
            }
        }
    };
    if (e <= 1) {
        take(s * total, e * total);
    } else {
        take(s * total, total);
        take(0, (e - 1) * total);
    }
    return out;
}

static QColor colorOf(const Vec &v, double alpha)
{
    if (v.size() < 3)
        return QColor(0, 0, 0, int(qBound(0.0, alpha, 1.0) * 255));
    const bool wide = v[0] > 1 || v[1] > 1 || v[2] > 1;
    const double k = wide ? 1 / 255.0 : 1.0;
    const double a = v.size() > 3 ? qBound(0.0, v[3] * (v[3] > 1 ? 1 / 255.0 : 1.0), 1.0) : 1.0;
    return QColor::fromRgbF(float(qBound(0.0, v[0] * k, 1.0)), float(qBound(0.0, v[1] * k, 1.0)), float(qBound(0.0, v[2] * k, 1.0)), float(qBound(0.0, a * alpha, 1.0)));
}

struct StyleOp
{
    const Item *item;
    QTransform toLayer;
    double opacity;
    QPainterPath path;
};

static void collect(const std::vector<Item> &items, double t, const QTransform &toLayer, double opacity, std::vector<Trim> trims, std::vector<size_t> styles, std::vector<StyleOp> &out)
{
    for (int n = int(items.size()) - 1; n >= 0; --n) {
        const Item &it = items[size_t(n)];
        if (it.hidden)
            continue;
        switch (it.kind) {
        case Kind::Fill:
        case Kind::Stroke:
        case Kind::GradientFill:
        case Kind::GradientStroke:
            out.push_back({ &it, toLayer, opacity, {} });
            styles.push_back(out.size() - 1);
            break;
        case Kind::Trim:
            trims.push_back({ it.start.scalar(t, 0) / 100.0, it.end.scalar(t, 100) / 100.0, it.offset.scalar(t, 0) / 360.0 });
            break;
        case Kind::Path:
        case Kind::Ellipse:
        case Kind::Rect:
        case Kind::Star: {
            if (styles.empty())
                break;
            QPainterPath p = it.kind == Kind::Path ? pathFrom(it.shape.at(t)) : it.kind == Kind::Ellipse ? ellipse(it, t) : it.kind == Kind::Rect ? rect(it, t) : star(it, t);
            for (const Trim &tr : trims)
                p = trimPath(p, tr);
            if (p.isEmpty())
                break;
            const QPainterPath mapped = toLayer.map(p);
            for (size_t s : styles)
                out[s].path.addPath(mapped);
            break;
        }
        case Kind::Group: {
            const double o = opacity * it.tr.opacity(t);
            if (o <= 0)
                break;
            collect(it.items, t, it.tr.matrix(t) * toLayer, o, trims, styles, out);
            break;
        }
        default:
            break;
        }
    }
}

static QBrush gradient(const Item &it, const StyleOp &op, double t)
{
    const QPointF a = op.toLayer.map(point(it.gStart.at(t)));
    const QPointF b = op.toLayer.map(point(it.gEnd.at(t)));
    QGradientStops stops;
    const Vec v = it.gStops.at(t);
    const int n = it.gCount;
    const bool alpha = int(v.size()) >= n * 4 + n * 2;
    for (int k = 0; k < n && size_t(k * 4 + 3) < v.size(); ++k) {
        QColor c = colorOf({ v[size_t(k * 4 + 1)], v[size_t(k * 4 + 2)], v[size_t(k * 4 + 3)] }, 1.0);
        if (alpha)
            c.setAlphaF(float(qBound(0.0, v[size_t(n * 4 + k * 2 + 1)], 1.0)));
        stops.append({ qBound(0.0, v[size_t(k * 4)], 1.0), c });
    }
    if (it.gType == 2) {
        QRadialGradient g(a, QLineF(a, b).length());
        g.setStops(stops);
        return QBrush(g);
    }
    QLinearGradient g(a, b);
    g.setStops(stops);
    return QBrush(g);
}

static void paintStyle(QPainter *p, const StyleOp &op, double t)
{
    const Item &it = *op.item;
    if (op.path.isEmpty())
        return;
    const double alpha = it.opacity.scalar(t, 100) / 100.0 * op.opacity;
    if (alpha <= 0)
        return;
    QPainterPath path = op.path;
    path.setFillRule(it.rule == 2 ? Qt::OddEvenFill : Qt::WindingFill);
    if (it.kind == Kind::Fill) {
        p->fillPath(path, colorOf(it.color.at(t), alpha));
        return;
    }
    if (it.kind == Kind::GradientFill) {
        const qreal saved = p->opacity();
        p->setOpacity(saved * alpha);
        p->fillPath(path, gradient(it, op, t));
        p->setOpacity(saved);
        return;
    }
    const double scale = std::sqrt(std::abs(op.toLayer.determinant()));
    const double raw = it.width.scalar(t, 1);
    const double w = raw * scale;
    if (w <= 0)
        return;
    QPen pen;
    pen.setWidthF(w);
    pen.setCapStyle(it.cap == 1 ? Qt::FlatCap : it.cap == 3 ? Qt::SquareCap : Qt::RoundCap);
    pen.setJoinStyle(it.join == 1 ? Qt::MiterJoin : it.join == 3 ? Qt::BevelJoin : Qt::RoundJoin);
    pen.setMiterLimit(it.miter);
    if (!it.dashes.empty() && raw > 0) {
        QList<qreal> pattern;
        qreal offset = 0;
        for (const auto &[name, value] : it.dashes) {
            const double v = value.scalar(t, 0) / raw;
            if (name == QLatin1String("o"))
                offset = v;
            else
                pattern.append(qMax(0.01, v));
        }
        if (pattern.size() % 2)
            pattern.append(pattern.last());
        if (!pattern.isEmpty()) {
            pen.setDashPattern(pattern);
            pen.setDashOffset(offset);
        }
    }
    if (it.kind == Kind::GradientStroke) {
        pen.setBrush(gradient(it, op, t));
        const qreal saved = p->opacity();
        p->setOpacity(saved * alpha);
        p->strokePath(path, pen);
        p->setOpacity(saved);
        return;
    }
    pen.setColor(colorOf(it.color.at(t), alpha));
    p->strokePath(path, pen);
}

class Renderer
{
public:
    explicit Renderer(const Composition &c)
        : m_comp(c)
    {
    }

    void layers(QPainter *p, const std::vector<Layer> &list, double t, const QTransform &base, double opacity) const
    {
        QHash<int, const Layer *> byInd;
        for (const Layer &l : list)
            byInd.insert(l.ind, &l);
        for (int n = int(list.size()) - 1; n >= 0; --n) {
            const Layer &l = list[size_t(n)];
            if (l.matteSource || l.hidden || t < l.ip || t >= l.op)
                continue;
            const Layer *matte = l.matte && n > 0 && list[size_t(n - 1)].matteSource ? &list[size_t(n - 1)] : nullptr;
            if (!matte) {
                layer(p, l, byInd, t, base, opacity);
                continue;
            }
            if (t < matte->ip || t >= matte->op) {
                if (l.matte == 2 || l.matte == 4)
                    layer(p, l, byInd, t, base, opacity);
                continue;
            }
            const QSize size(p->device()->width(), p->device()->height());
            QImage content(size, QImage::Format_ARGB32_Premultiplied);
            content.fill(Qt::transparent);
            QImage mask(size, QImage::Format_ARGB32_Premultiplied);
            mask.fill(Qt::transparent);
            {
                QPainter cp(&content);
                cp.setRenderHints(p->renderHints());
                layer(&cp, l, byInd, t, base, opacity);
            }
            {
                QPainter mp(&mask);
                mp.setRenderHints(p->renderHints());
                layer(&mp, *matte, byInd, t, base, 1.0);
            }
            if (l.matte == 3 || l.matte == 4) {
                for (int y = 0; y < mask.height(); ++y) {
                    QRgb *row = reinterpret_cast<QRgb *>(mask.scanLine(y));
                    for (int x = 0; x < mask.width(); ++x) {
                        const QRgb c = row[x];
                        const int luma = (qRed(c) * 54 + qGreen(c) * 183 + qBlue(c) * 19) >> 8;
                        row[x] = qRgba(0, 0, 0, luma);
                    }
                }
            }
            {
                QPainter cp(&content);
                cp.setCompositionMode(l.matte == 2 || l.matte == 4 ? QPainter::CompositionMode_DestinationOut : QPainter::CompositionMode_DestinationIn);
                cp.drawImage(0, 0, mask);
            }
            p->save();
            p->resetTransform();
            p->setOpacity(1);
            p->drawImage(0, 0, content);
            p->restore();
        }
    }

private:
    QTransform world(const Layer &l, const QHash<int, const Layer *> &byInd, double t) const
    {
        QTransform m = l.ks.matrix(t - l.st);
        const Layer *cur = &l;
        for (int guard = 0; cur->parent >= 0 && guard < 64; ++guard) {
            const Layer *parent = byInd.value(cur->parent);
            if (!parent)
                break;
            m = m * parent->ks.matrix(t - parent->st);
            cur = parent;
        }
        return m;
    }

    void layer(QPainter *p, const Layer &l, const QHash<int, const Layer *> &byInd, double t, const QTransform &base, double opacity) const
    {
        const double lt = t - l.st;
        const double o = opacity * l.ks.opacity(lt);
        if (o <= 0 || l.type == 3)
            return;
        const QTransform m = world(l, byInd, t) * base;
        if (!m.isInvertible())
            return;
        p->save();
        p->setTransform(m);
        p->setOpacity(o);
        if (!l.masks.empty()) {
            QPainterPath clip;
            bool first = true;
            for (const Mask &mk : l.masks) {
                QPainterPath mp = pathFrom(mk.path.at(lt));
                if (mk.inverted) {
                    QPainterPath all;
                    all.addRect(QRectF(-1e5, -1e5, 2e5, 2e5));
                    mp = all.subtracted(mp);
                }
                if (mk.mode == QLatin1String("n"))
                    continue;
                if (first) {
                    clip = mk.mode == QLatin1String("s") ? QPainterPath() : mp;
                    first = false;
                } else if (mk.mode == QLatin1String("a")) {
                    clip = clip.united(mp);
                } else if (mk.mode == QLatin1String("s")) {
                    clip = clip.subtracted(mp);
                } else if (mk.mode == QLatin1String("i")) {
                    clip = clip.intersected(mp);
                }
            }
            if (!first)
                p->setClipPath(clip, Qt::IntersectClip);
        }
        switch (l.type) {
        case 4: {
            std::vector<StyleOp> ops;
            collect(l.shapes, lt, QTransform(), 1.0, {}, {}, ops);
            for (const StyleOp &op : ops)
                paintStyle(p, op, lt);
            break;
        }
        case 0: {
            const auto it = m_comp.assets.constFind(l.ref);
            if (it == m_comp.assets.constEnd())
                break;
            const double ct = l.remap.present ? l.remap.scalar(lt, 0) * m_comp.fr : lt / l.sr;
            if (l.w > 0 && l.h > 0)
                p->setClipRect(QRectF(0, 0, l.w, l.h), Qt::IntersectClip);
            layers(p, it->layers, ct, m, o);
            break;
        }
        case 1:
            if (l.solid.isValid())
                p->fillRect(QRectF(0, 0, l.sw, l.sh), l.solid);
            break;
        case 2: {
            const auto it = m_comp.assets.constFind(l.ref);
            if (it != m_comp.assets.constEnd() && !it->image.isNull())
                p->drawImage(QPointF(0, 0), it->image);
            break;
        }
        default:
            break;
        }
        p->restore();
    }

    const Composition &m_comp;
};

static void render(const Composition &c, QPainter *p, double frame, const QRectF &box)
{
    const double s = qMin(box.width() / c.w, box.height() / c.h);
    if (s <= 0)
        return;
    QTransform fit;
    fit.translate(box.x() + (box.width() - c.w * s) / 2, box.y() + (box.height() - c.h * s) / 2);
    fit.scale(s, s);
    p->setRenderHints(QPainter::Antialiasing | QPainter::SmoothPixmapTransform);
    Renderer(c).layers(p, c.layers, frame, fit * p->worldTransform(), 1.0);
}

static QMutex cacheLock;

static std::shared_ptr<const Composition> openComposition(const QString &path, QString *error)
{
    static QHash<QString, std::weak_ptr<const Composition>> cache;
    QMutexLocker l(&cacheLock);
    if (auto hit = cache.value(path).lock())
        return hit;
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly)) {
        *error = f.errorString();
        return nullptr;
    }
    std::shared_ptr<const Composition> c = parse(f.readAll(), error);
    if (c)
        cache.insert(path, c);
    return c;
}

}

LottieAnimation::LottieAnimation(QQuickItem *parent)
    : QQuickPaintedItem(parent)
{
    setAntialiasing(true);
    m_timer.setTimerType(Qt::PreciseTimer);
    connect(&m_timer, &QTimer::timeout, this, &LottieAnimation::tick);
}

LottieAnimation::~LottieAnimation() = default;

void LottieAnimation::setSource(const QUrl &source)
{
    if (m_source == source)
        return;
    m_source = source;
    emit sourceChanged();
    load();
}

void LottieAnimation::setPlaying(bool playing)
{
    if (m_playing == playing)
        return;
    m_playing = playing;
    emit playingChanged();
    if (playing && m_ended)
        restart();
    sync();
}

void LottieAnimation::setLoops(int loops)
{
    if (m_loops == loops)
        return;
    m_loops = loops;
    emit loopsChanged();
}

void LottieAnimation::setSpeed(qreal speed)
{
    if (qFuzzyCompare(m_speed, speed) || speed <= 0)
        return;
    m_speed = speed;
    emit speedChanged();
}

void LottieAnimation::setStillProgress(qreal progress)
{
    progress = qBound(0.0, progress, 1.0);
    if (qFuzzyCompare(m_still, progress))
        return;
    m_still = progress;
    emit stillProgressChanged();
    if (!m_playing)
        showStill();
}

qreal LottieAnimation::progress() const
{
    if (!m_comp)
        return 0;
    return qBound(0.0, (m_frame - m_comp->ip) / (m_comp->op - m_comp->ip), 1.0);
}

int LottieAnimation::durationMs() const
{
    return m_comp ? int((m_comp->op - m_comp->ip) / m_comp->fr * 1000) : 0;
}

void LottieAnimation::load()
{
    m_timer.stop();
    m_comp.reset();
    m_done = 0;
    m_ended = false;
    Status next = Null;
    if (!m_source.isEmpty()) {
        QString error;
        m_comp = lottie::openComposition(QQmlFile::urlToLocalFileOrQrc(m_source), &error);
        if (m_comp) {
            next = Ready;
            m_frame = m_comp->ip;
            setImplicitSize(m_comp->w, m_comp->h);
        } else {
            next = Error;
            qCWarning(lcLottie) << "can't play" << m_source.toString() << error;
        }
    }
    m_status = next;
    emit statusChanged();
    if (!m_playing)
        showStill();
    sync();
    update();
}

void LottieAnimation::restart()
{
    m_done = 0;
    m_ended = false;
    if (m_comp)
        m_frame = m_comp->ip;
    emit progressChanged();
    sync();
    update();
}

void LottieAnimation::seek(qreal progress)
{
    if (!m_comp)
        return;
    m_frame = m_comp->ip + qBound(0.0, progress, 1.0) * (m_comp->op - m_comp->ip - 0.001);
    emit progressChanged();
    update();
}

void LottieAnimation::showStill()
{
    if (!m_comp)
        return;
    m_frame = m_comp->ip + m_still * (m_comp->op - m_comp->ip - 0.001);
    emit progressChanged();
    update();
}

void LottieAnimation::sync()
{
    const bool want = m_comp && m_playing && !m_ended && isVisible() && window() && window()->isVisible();
    if (want && !m_timer.isActive()) {
        m_timer.setInterval(qMax(16, int(1000.0 / qMin(30.0, m_comp->fr))));
        m_clock.start();
        m_timer.start();
    } else if (!want && m_timer.isActive()) {
        m_timer.stop();
    }
}

void LottieAnimation::tick()
{
    if (!m_comp) {
        m_timer.stop();
        return;
    }
    const double len = m_comp->op - m_comp->ip;
    double f = m_frame + m_clock.restart() / 1000.0 * m_comp->fr * m_speed;
    if (f >= m_comp->op) {
        if (m_loops < 0 || ++m_done < m_loops) {
            f = m_comp->ip + std::fmod(f - m_comp->ip, len);
        } else {
            m_frame = m_comp->op - 0.001;
            m_ended = true;
            m_timer.stop();
            emit progressChanged();
            update();
            emit finished();
            return;
        }
    }
    m_frame = f;
    emit progressChanged();
    update();
}

void LottieAnimation::itemChange(ItemChange change, const ItemChangeData &value)
{
    QQuickPaintedItem::itemChange(change, value);
    if (change == ItemSceneChange) {
        disconnect(m_windowWatch);
        if (value.window)
            m_windowWatch = connect(value.window, &QWindow::visibleChanged, this, &LottieAnimation::sync);
        sync();
    } else if (change == ItemVisibleHasChanged) {
        sync();
    }
}

void LottieAnimation::paint(QPainter *painter)
{
    if (!m_comp)
        return;
    lottie::render(*m_comp, painter, m_frame, QRectF(0, 0, width(), height()));
}

QImage LottieAnimation::renderFrame(const QString &path, qreal progress, const QSize &size)
{
    QString error;
    const auto c = lottie::openComposition(path, &error);
    QImage img(size, QImage::Format_ARGB32_Premultiplied);
    img.fill(Qt::transparent);
    if (!c)
        return img;
    QPainter p(&img);
    lottie::render(*c, &p, c->ip + qBound(0.0, progress, 1.0) * (c->op - c->ip - 0.001), QRectF(QPointF(0, 0), QSizeF(size)));
    return img;
}
