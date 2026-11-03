struct Pair {
    int lo;
    int hi;
};

/* Write two fields through a record pointer. */
void spread(struct Pair *p, int delta) {
    p->lo += delta;
    p->hi -= delta;
}

/* Read through a const record pointer. */
int width(const struct Pair *p) {
    return p->hi - p->lo;
}

/* A local record behind a pointer local. */
int span(void) {
    struct Pair p;
    p.lo = 3;
    p.hi = 11;
    struct Pair *q = &p;
    return q->hi - q->lo;
}

/* A record local passed by address to a callee that writes its fields. */
int spread_through(int lo, int hi, int delta) {
    struct Pair p;
    p.lo = lo;
    p.hi = hi;
    spread(&p, delta);
    return p.lo * 10 + p.hi;
}
