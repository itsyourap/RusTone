/*
 * Custom headers: a module that adds, replaces or removes headers in every
 * request the main account sends, including the ones pjsip makes by itself
 * (PRACK, ACK, BYE, CANCEL, authentication retries, re-registrations).
 *
 * The rules come from Rust as text, one per line:
 *     METHODS \t NAME \t VALUE \n
 * METHODS is "*" or space-separated method names; an empty VALUE removes
 * the header. A request is the main account's when its From host is the
 * one given (or anonymous, with the caller ID hidden).
 */
#include <pjsua-lib/pjsua.h>
#include <string.h>

typedef struct jf_hdr_rule {
    pj_str_t methods;   /* "*" or " INVITE PRACK " (spaces around each) */
    pj_str_t name;
    pj_str_t value;     /* empty: remove */
} jf_hdr_rule;

static pj_pool_t *jf_hdr_pool;          /* the mutex, while installed */
static pj_pool_t *jf_hdr_rules_pool;    /* the current rules          */
static pj_mutex_t *jf_hdr_mutex;
static jf_hdr_rule *jf_hdr_rules;
static unsigned jf_hdr_count;
static pj_str_t jf_hdr_from_host;

/* Whether the request's method is one of the rule's. */
static pj_bool_t jf_hdr_method_matches(const jf_hdr_rule *rule,
                                       const pjsip_method *method)
{
    char needle[48];
    pj_str_t s;

    if (rule->methods.slen == 1 && rule->methods.ptr[0] == '*')
        return PJ_TRUE;
    if (method->name.slen + 3 > (pj_ssize_t)sizeof(needle))
        return PJ_FALSE;
    pj_ansi_snprintf(needle, sizeof(needle), " %.*s ",
                     (int)method->name.slen, method->name.ptr);
    s = pj_str(needle);
    return pj_stristr(&rule->methods, &s) != NULL;
}

/* Whether the request is the main account's. */
static pj_bool_t jf_hdr_is_ours(pjsip_tx_data *tdata)
{
    static const pj_str_t ANON = { "anonymous.invalid", 17 };
    pjsip_from_hdr *from;
    pjsip_uri *uri;
    pjsip_sip_uri *sip;

    from = (pjsip_from_hdr*)pjsip_msg_find_hdr(tdata->msg, PJSIP_H_FROM, NULL);
    if (!from)
        return PJ_FALSE;
    uri = (pjsip_uri*)pjsip_uri_get_uri(from->uri);
    if (!PJSIP_URI_SCHEME_IS_SIP(uri) && !PJSIP_URI_SCHEME_IS_SIPS(uri))
        return PJ_FALSE;
    sip = (pjsip_sip_uri*)uri;
    return pj_stricmp(&sip->host, &jf_hdr_from_host) == 0 ||
           pj_stricmp(&sip->host, &ANON) == 0;
}

static pj_status_t jf_hdr_on_tx_request(pjsip_tx_data *tdata)
{
    unsigned i;
    pj_bool_t changed = PJ_FALSE;

    if (!jf_hdr_mutex)
        return PJ_SUCCESS;
    pj_mutex_lock(jf_hdr_mutex);
    if (jf_hdr_count && jf_hdr_is_ours(tdata)) {
        for (i = 0; i < jf_hdr_count; ++i) {
            const jf_hdr_rule *rule = &jf_hdr_rules[i];
            pjsip_hdr *h;

            if (!jf_hdr_method_matches(rule, &tdata->msg->line.req.method))
                continue;
            /* Replace: drop the header wherever it is, then add ours. */
            while ((h = (pjsip_hdr*)pjsip_msg_find_hdr_by_name(
                            tdata->msg, &rule->name, NULL)) != NULL) {
                pj_list_erase(h);
                changed = PJ_TRUE;
            }
            if (rule->value.slen) {
                h = (pjsip_hdr*)pjsip_generic_string_hdr_create(
                        tdata->pool, &rule->name, &rule->value);
                pjsip_msg_add_hdr(tdata->msg, h);
                changed = PJ_TRUE;
            }
        }
    }
    pj_mutex_unlock(jf_hdr_mutex);
    /* A message sent before (another server address, after a failure) is
     * printed again. */
    if (changed)
        pjsip_tx_data_invalidate_msg(tdata);
    return PJ_SUCCESS;
}

static pjsip_module jf_hdr_module = {
    NULL, NULL,                                 /* prev, next          */
    { "mod-rustone-headers", 19 },              /* name                */
    -1,                                         /* id                  */
    PJSIP_MOD_PRIORITY_APPLICATION,             /* priority: before the
                                                   message is printed  */
    NULL, NULL, NULL, NULL,                     /* load/start/stop/unl */
    NULL, NULL,                                 /* on_rx_*             */
    &jf_hdr_on_tx_request,                      /* on_tx_request       */
    NULL,                                       /* on_tx_response      */
    NULL,                                       /* on_tsx_state        */
};

/* Registers the module on pjsua's endpoint (after pjsua_init). */
pj_status_t jf_headers_install(void)
{
    pj_status_t status;

    jf_hdr_pool = pjsua_pool_create("jfhdr", 512, 512);
    if (!jf_hdr_pool)
        return PJ_ENOMEM;
    status = pj_mutex_create_simple(jf_hdr_pool, "jfhdr", &jf_hdr_mutex);
    if (status != PJ_SUCCESS)
        return status;
    jf_hdr_module.id = -1;
    return pjsip_endpt_register_module(pjsua_get_pjsip_endpt(), &jf_hdr_module);
}

/* Unregisters it and frees the rules (before pjsua_destroy). */
void jf_headers_remove(void)
{
    if (jf_hdr_module.id != -1)
        pjsip_endpt_unregister_module(pjsua_get_pjsip_endpt(), &jf_hdr_module);
    if (jf_hdr_mutex)
        pj_mutex_destroy(jf_hdr_mutex);
    jf_hdr_mutex = NULL;
    if (jf_hdr_rules_pool)
        pj_pool_release(jf_hdr_rules_pool);
    jf_hdr_rules_pool = NULL;
    if (jf_hdr_pool)
        pj_pool_release(jf_hdr_pool);
    jf_hdr_pool = NULL;
    jf_hdr_rules = NULL;
    jf_hdr_count = 0;
}

/* Copies `len` bytes of `p` into `pool` as a pj_str_t. */
static pj_str_t jf_hdr_dup(pj_pool_t *pool, const char *p, pj_size_t len)
{
    pj_str_t in, out;

    in.ptr = (char*)p;
    in.slen = (pj_ssize_t)len;
    pj_strdup(pool, &out, &in);
    return out;
}

/* Replaces the rules: `rules` as described above (NUL-terminated), for
 * requests whose From host is `from_host`. Empty `rules` turns it off. */
pj_status_t jf_headers_set(const char *from_host, const char *rules)
{
    pj_pool_t *pool, *old;
    jf_hdr_rule *list;
    unsigned count = 0, max = 0;
    const char *p, *end;

    if (!jf_hdr_mutex)
        return PJ_EINVALIDOP;
    for (p = rules; *p; ++p)
        if (*p == '\n')
            ++max;
    pool = pjsua_pool_create("jfhdr", 1024, 512);
    if (!pool)
        return PJ_ENOMEM;
    list = (jf_hdr_rule*)pj_pool_calloc(pool, max ? max : 1, sizeof(jf_hdr_rule));
    for (p = rules; (end = strchr(p, '\n')) != NULL; p = end + 1) {
        const char *t1 = (const char*)memchr(p, '\t', end - p);
        const char *t2 = t1 ? (const char*)memchr(t1 + 1, '\t', end - t1 - 1)
                            : NULL;
        jf_hdr_rule *rule;

        if (!t1 || !t2 || count == max)
            continue;
        rule = &list[count++];
        if (t1 - p == 1 && *p == '*') {
            rule->methods = jf_hdr_dup(pool, "*", 1);
        } else {
            /* Padded with spaces, so " INVITE " finds whole names. */
            char *m = (char*)pj_pool_alloc(pool, (t1 - p) + 3);

            pj_ansi_snprintf(m, (t1 - p) + 3, " %.*s ", (int)(t1 - p), p);
            rule->methods.ptr = m;
            rule->methods.slen = (t1 - p) + 2;
        }
        rule->name = jf_hdr_dup(pool, t1 + 1, t2 - t1 - 1);
        rule->value = jf_hdr_dup(pool, t2 + 1, end - t2 - 1);
    }

    pj_mutex_lock(jf_hdr_mutex);
    old = jf_hdr_rules_pool;
    jf_hdr_rules_pool = pool;
    jf_hdr_rules = list;
    jf_hdr_count = count;
    jf_hdr_from_host = jf_hdr_dup(pool, from_host, strlen(from_host));
    pj_mutex_unlock(jf_hdr_mutex);
    if (old)
        pj_pool_release(old);
    return PJ_SUCCESS;
}
