struct point {
    int x;
    int y;
};

struct rect {
    struct point origin;
    struct point corner;
};

struct point make_point(int x, int y) {
    struct point p;
    p.x = x;
    p.y = y;
    return p;
}

int manhattan(struct point p) {
    int x = p.x < 0 ? -p.x : p.x;
    int y = p.y < 0 ? -p.y : p.y;
    return x + y;
}

struct point origin(void) {
    struct point p = {0, 0};
    p.x = 1;
    p.y = 2;
    return p;
}

int rect_area(struct rect r) {
    int width = r.corner.x - r.origin.x;
    int height = r.corner.y - r.origin.y;
    return width * height;
}

struct rect make_rect(int x0, int y0, int x1, int y1) {
    struct rect r = {{x0, y0}, {x1, y1}};
    return r;
}

int field_sum(struct point a, struct point b) {
    struct point total = {a.x + b.x, a.y + b.y};
    return total.x * 10 + total.y;
}
