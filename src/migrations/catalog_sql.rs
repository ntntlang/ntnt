//! Fixed, pg_catalog-qualified introspection only. Never prepare application SQL
//! on the target, evaluate a stored expression, or select application rows.
//!
//! Every query in `QUERIES` takes the same three parameters:
//! - `$1` (text): the managed schema. Objects there get unqualified catalog
//!   keys (`relation:accounts`).
//! - `$2` (text[]): the managed scope: the managed schema and the schemas the
//!   history creates (`CapturedInputs::history_schemas`). Each is inspected
//!   whole, and a reference into one is covered.
//! - `$3` (text[]): objects the history creates in other schemas, by catalog
//!   key (`public.relation:audit_log`; `OUTSIDE_OBJECTS` finds them in
//!   replay). Each is inspected on its own: a relation with its columns,
//!   constraints, indexes, triggers, policies and rules, which follow it by
//!   its key. A reference to one is covered. Nothing else in those schemas is
//!   inspected.
//!
//! - `$4` (text[]), for the queries `WITH_DECLARED_EXTENSIONS` names only:
//!   the declared extensions (`[databases.<name>] extensions`, #430), which
//!   a superuser installed. Each is covered wherever it is installed, but in
//!   `_ntnt` and PostgreSQL's schemas, by its version and schema, and the
//!   relations it installs in the scope are covered with it. Its members
//!   outside the scope are not inspected. Empty, these queries return what
//!   they did before `$4` was added.
//!
//! Each returns the object's name, its definition, whether it is covered, and
//! the schema that qualifies its catalog key: NULL in the managed schema and
//! for database-wide objects, `quote_ident(schema)` anywhere else. With `$2`
//! the managed schema alone and `$3` empty, the queries return exactly what
//! they did before either was added, so catalogs and plan digests of
//! histories that stay in the managed schema are unchanged.
// `DECLARED_OWNED`, with the declared extensions in parameter `$p`.
macro_rules! declared_owned {
    ($p:literal) => {
        concat!(
            r#"owned(classid, objid) AS (
  SELECT d.classid, d.objid FROM pg_catalog.pg_depend d JOIN pg_catalog.pg_extension e ON e.oid=d.refobjid
   WHERE d.refclassid='pg_catalog.pg_extension'::pg_catalog.regclass AND d.deptype='e' AND e.extname=ANY("#,
            $p,
            r#"::text[])
  UNION
  SELECT d.classid, d.objid FROM owned o JOIN pg_catalog.pg_depend d ON d.refclassid=o.classid AND d.refobjid=o.objid
   WHERE d.deptype IN ('i','a'))"#
        )
    };
}

pub(super) const QUERIES: &[(&str, &str)] = &[
    (
        "schema",
        r#"
SELECT n.nspname::text, jsonb_build_object('owner',pg_get_userbyid(n.nspowner),
 'grants',(SELECT jsonb_agg(jsonb_build_object('grantor',pg_get_userbyid(a.grantor),'grantee',CASE WHEN a.grantee=0 THEN jsonb_build_object('public',true) ELSE to_jsonb(pg_get_userbyid(a.grantee)) END,'privilege',a.privilege_type,'grantable',a.is_grantable) ORDER BY a.grantor,a.grantee,a.privilege_type)
 FROM aclexplode(coalesce(n.nspacl,acldefault('n',n.nspowner))) a)),
 has_schema_privilege(n.oid,'USAGE'),NULL::text
FROM pg_catalog.pg_namespace n WHERE n.nspname=$1 OR n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND 'schema:'||n.nspname=ANY($3))
"#,
    ),
    (
        "relation",
        concat!(
            "\nWITH RECURSIVE ",
            declared_owned!("$4"),
            r#",
owned_rel AS (SELECT o.objid FROM owned o WHERE o.classid='pg_catalog.pg_class'::pg_catalog.regclass)
-- What the declared extensions ($4) own keeps its installer's owner and
-- grants, which are not compared, as for an extension's routines and types.
SELECT c.relname::text, jsonb_build_object('kind',c.relkind,'owner',CASE WHEN c.oid IN (SELECT objid FROM owned_rel) THEN NULL ELSE pg_get_userbyid(c.relowner) END,
 'persistence',c.relpersistence,'row_security',c.relrowsecurity,'force_row_security',c.relforcerowsecurity,
 'partition',c.relispartition,'replica_identity',c.relreplident,'access_method',am.amname,
 'options',c.reloptions,'tablespace',ts.spcname,'comment',obj_description(c.oid,'pg_class'),
 'partition_key',CASE WHEN c.relkind='p' THEN pg_get_partkeydef(c.oid) END,'partition_bound',pg_get_expr(c.relpartbound,c.oid),
 'partition_of',(SELECT pc.relname FROM pg_catalog.pg_inherits h JOIN pg_catalog.pg_class pc ON pc.oid=h.inhparent WHERE h.inhrelid=c.oid AND c.relispartition),
 'grants',CASE WHEN c.relkind IN ('i','I') OR c.oid IN (SELECT objid FROM owned_rel) THEN NULL ELSE (SELECT jsonb_agg(jsonb_build_object('grantor',pg_get_userbyid(a.grantor),'grantee',CASE WHEN a.grantee=0 THEN jsonb_build_object('public',true) ELSE to_jsonb(pg_get_userbyid(a.grantee)) END,'privilege',a.privilege_type,'grantable',a.is_grantable) ORDER BY a.grantor,a.grantee,a.privilege_type)
 FROM aclexplode(coalesce(c.relacl,acldefault(CASE WHEN c.relkind='S' THEN 's'::"char" ELSE 'r'::"char" END,c.relowner))) a) END)
 -- A composite type's privileges and comment are its pg_type row's.
 ||CASE WHEN c.relkind='c' THEN (SELECT jsonb_build_object('grants',CASE WHEN c.oid IN (SELECT objid FROM owned_rel) THEN NULL ELSE (SELECT jsonb_agg(jsonb_build_object('grantor',pg_get_userbyid(a.grantor),'grantee',CASE WHEN a.grantee=0 THEN jsonb_build_object('public',true) ELSE to_jsonb(pg_get_userbyid(a.grantee)) END,'privilege',a.privilege_type,'grantable',a.is_grantable) ORDER BY a.grantor,a.grantee,a.privilege_type)
 FROM aclexplode(coalesce(ct.typacl,acldefault('T',ct.typowner))) a) END,
 'comment',obj_description(ct.oid,'pg_type')) FROM pg_catalog.pg_type ct WHERE ct.oid=c.reltype) ELSE '{}'::jsonb END,
 ((c.relkind='r' AND am.amname='heap') OR (c.relkind IN ('i','I') AND am.oid<16384) OR c.relkind IN ('p','v','S','c'))
 AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_inherits h JOIN pg_catalog.pg_class ch ON ch.oid=h.inhrelid JOIN pg_catalog.pg_class pa ON pa.oid=h.inhparent
   WHERE (h.inhrelid=c.oid OR h.inhparent=c.oid) AND (NOT ch.relispartition OR ch.relnamespace<>c.relnamespace OR pa.relnamespace<>c.relnamespace))
 AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_depend xd JOIN pg_catalog.pg_extension xe ON xe.oid=xd.refobjid WHERE xd.classid='pg_catalog.pg_class'::regclass AND xd.objid=c.oid AND xd.deptype='e' AND xe.extname<>ALL($4)),
 CASE WHEN n.nspname<>$1 THEN quote_ident(n.nspname) END
FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
LEFT JOIN pg_catalog.pg_am am ON am.oid=c.relam LEFT JOIN pg_catalog.pg_tablespace ts ON ts.oid=c.reltablespace
-- An index follows its table.
LEFT JOIN pg_catalog.pg_index x ON x.indexrelid=c.oid LEFT JOIN pg_catalog.pg_class xt ON xt.oid=x.indrelid
WHERE n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.relation:'||coalesce(xt.relname,c.relname)=ANY($3)) ORDER BY c.relname LIMIT 10001
"#
        ),
    ),
    (
        "column",
        r#"
SELECT quote_ident(c.relname)||'.'||quote_ident(a.attname), jsonb_build_object('type',format_type(a.atttypid,a.atttypmod),
 'type_schema',tn.nspname,'dimensions',a.attndims,'ordinal',a.attnum,'not_null',a.attnotnull,
 'default',pg_get_expr(d.adbin,d.adrelid),'identity',a.attidentity,'generated',a.attgenerated,
 'collation',CASE WHEN a.attcollation=0 THEN NULL ELSE quote_ident(cn.nspname)||'.'||quote_ident(co.collname) END,
 'collation_version',co.collversion,'storage',a.attstorage,'compression',a.attcompression,
 'comment',col_description(c.oid,a.attnum),
 'grants',(SELECT jsonb_agg(jsonb_build_object('grantor',pg_get_userbyid(x.grantor),'grantee',CASE WHEN x.grantee=0 THEN jsonb_build_object('public',true) ELSE to_jsonb(pg_get_userbyid(x.grantee)) END,'privilege',x.privilege_type,'grantable',x.is_grantable) ORDER BY x.grantor,x.grantee,x.privilege_type) FROM aclexplode(a.attacl) x)),
 -- The type's own definition is inspected wherever it is covered: in pg_catalog,
 -- in the scope, or as an object the history created outside it (an array
 -- by its element type, a composite type by its relation).
 (tn.nspname='pg_catalog' OR tn.nspname=ANY($2) OR (cardinality($3::text[])>0 AND (SELECT quote_ident(rn.nspname)||CASE WHEN rt.typrelid<>0 THEN '.relation:'||rc.relname ELSE '.type:'||rt.typname END
   FROM pg_catalog.pg_type rt JOIN pg_catalog.pg_namespace rn ON rn.oid=rt.typnamespace LEFT JOIN pg_catalog.pg_class rc ON rc.oid=rt.typrelid
   WHERE rt.oid=CASE WHEN t.typelem<>0 AND t.typlen=-1 THEN t.typelem ELSE t.oid END)=ANY($3)))
 AND (a.attcollation=0 OR cn.nspname='pg_catalog') AND NOT EXISTS (
 SELECT 1 FROM pg_catalog.pg_depend dep JOIN pg_catalog.pg_proc p ON p.oid=dep.refobjid JOIN pg_catalog.pg_namespace pn ON pn.oid=p.pronamespace
 WHERE dep.classid='pg_catalog.pg_attrdef'::regclass AND dep.objid=d.oid AND dep.refclassid='pg_catalog.pg_proc'::regclass AND pn.nspname<>'pg_catalog' AND pn.nspname<>ALL($2)
 AND (cardinality($3::text[])=0 OR quote_ident(pn.nspname)||'.routine:'||p.proname||'('||pg_get_function_identity_arguments(p.oid)||')'<>ALL($3))),
 CASE WHEN n.nspname<>$1 THEN quote_ident(n.nspname) END
FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
JOIN pg_catalog.pg_type t ON t.oid=a.atttypid JOIN pg_catalog.pg_namespace tn ON tn.oid=t.typnamespace
LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=c.oid AND d.adnum=a.attnum
LEFT JOIN pg_catalog.pg_collation co ON co.oid=a.attcollation LEFT JOIN pg_catalog.pg_namespace cn ON cn.oid=co.collnamespace
WHERE (n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.relation:'||c.relname=ANY($3))) AND c.relkind NOT IN ('i','I','S') AND a.attnum>0 AND NOT a.attisdropped ORDER BY c.relname,a.attnum LIMIT 10001
"#,
    ),
    (
        "constraint",
        r#"
SELECT quote_ident(c.relname)||'.'||quote_ident(k.conname), jsonb_build_object('kind',k.contype,'definition',pg_get_constraintdef(k.oid,false),
 'validated',k.convalidated,'deferrable',k.condeferrable,'deferred',k.condeferred,'local',k.conislocal,'no_inherit',k.connoinherit,
 'enforced',pg_catalog.to_jsonb(k)->'conenforced',
 'enforcement_triggers',(SELECT jsonb_agg(jsonb_build_object('schema',tn.nspname,'table',tc.relname,'function',p.proname,'events',t.tgtype,'enabled',t.tgenabled,'deferrable',t.tgdeferrable,'deferred',t.tginitdeferred) ORDER BY tn.nspname,tc.relname,p.proname,t.tgtype,t.tgenabled)
 FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_class tc ON tc.oid=t.tgrelid JOIN pg_catalog.pg_namespace tn ON tn.oid=tc.relnamespace JOIN pg_catalog.pg_proc p ON p.oid=t.tgfoid
 WHERE t.tgconstraint=k.oid AND t.tgisinternal)),
 k.contype IN ('p','u','c','f','n','t','x') AND (k.confrelid=0 OR fn.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(fn.nspname)||'.relation:'||fc.relname=ANY($3)))
 -- An exclusion constraint's operators (conexclop is NULL for every other kind).
 AND NOT EXISTS(SELECT 1 FROM unnest(k.conexclop) x JOIN pg_catalog.pg_operator eo ON eo.oid=x JOIN pg_catalog.pg_namespace eon ON eon.oid=eo.oprnamespace WHERE eon.nspname<>'pg_catalog' AND eon.nspname<>ALL($2)
 AND (cardinality($3::text[])=0 OR quote_ident(eon.nspname)||'.other:operator.'||eo.oprname||'('||CASE WHEN eo.oprleft=0 THEN 'NONE' ELSE format_type(eo.oprleft,NULL) END||','||CASE WHEN eo.oprright=0 THEN 'NONE' ELSE format_type(eo.oprright,NULL) END||')'<>ALL($3)))
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_depend dep JOIN pg_catalog.pg_proc p ON p.oid=dep.refobjid JOIN pg_catalog.pg_namespace pn ON pn.oid=p.pronamespace WHERE dep.classid='pg_catalog.pg_constraint'::regclass AND dep.objid=k.oid AND dep.refclassid='pg_catalog.pg_proc'::regclass AND pn.nspname<>'pg_catalog' AND pn.nspname<>ALL($2)
 AND (cardinality($3::text[])=0 OR quote_ident(pn.nspname)||'.routine:'||p.proname||'('||pg_get_function_identity_arguments(p.oid)||')'<>ALL($3))),
 CASE WHEN n.nspname<>$1 THEN quote_ident(n.nspname) END
FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_class c ON c.oid=k.conrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
LEFT JOIN pg_catalog.pg_class fc ON fc.oid=k.confrelid LEFT JOIN pg_catalog.pg_namespace fn ON fn.oid=fc.relnamespace
WHERE n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.relation:'||c.relname=ANY($3)) ORDER BY c.relname,k.conname LIMIT 10001
"#,
    ),
    (
        "index",
        r#"
SELECT c.relname::text,jsonb_build_object('table',i.indrelid::regclass::text,'definition',pg_get_indexdef(i.indexrelid,0,false),'valid',i.indisvalid,
 'ready',i.indisready,'live',i.indislive,'unique',i.indisunique,'primary',i.indisprimary,'exclusion',i.indisexclusion,
 'immediate',i.indimmediate,'clustered',i.indisclustered,'replica_identity',i.indisreplident,'nulls_not_distinct',i.indnullsnotdistinct),
 am.oid<16384 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_depend dep JOIN pg_catalog.pg_proc p ON p.oid=dep.refobjid JOIN pg_catalog.pg_namespace pn ON pn.oid=p.pronamespace WHERE dep.classid='pg_catalog.pg_class'::regclass AND dep.objid=i.indexrelid AND dep.refclassid='pg_catalog.pg_proc'::regclass AND pn.nspname<>'pg_catalog' AND pn.nspname<>ALL($2)
 AND (cardinality($3::text[])=0 OR quote_ident(pn.nspname)||'.routine:'||p.proname||'('||pg_get_function_identity_arguments(p.oid)||')'<>ALL($3)))
 AND NOT EXISTS(SELECT 1 FROM unnest(i.indclass) x JOIN pg_catalog.pg_opclass o ON o.oid=x JOIN pg_catalog.pg_namespace onsp ON onsp.oid=o.opcnamespace JOIN pg_catalog.pg_am oam ON oam.oid=o.opcmethod
 WHERE onsp.nspname<>'pg_catalog' AND onsp.nspname<>ALL($2) AND (cardinality($3::text[])=0 OR quote_ident(onsp.nspname)||'.other:operator_class.'||quote_ident(o.opcname)||' USING '||quote_ident(oam.amname)<>ALL($3))),
 CASE WHEN n.nspname<>$1 THEN quote_ident(n.nspname) END
FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class c ON c.oid=i.indexrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
JOIN pg_catalog.pg_class t ON t.oid=i.indrelid
JOIN pg_catalog.pg_am am ON am.oid=c.relam WHERE n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.relation:'||t.relname=ANY($3)) ORDER BY c.relname LIMIT 10001
"#,
    ),
    (
        "view",
        r#"
SELECT c.relname::text,jsonb_build_object('kind',c.relkind,'definition',pg_get_viewdef(c.oid,false)),
 c.relkind='v' AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_rewrite r JOIN pg_catalog.pg_depend dep ON dep.classid='pg_catalog.pg_rewrite'::regclass AND dep.objid=r.oid
   LEFT JOIN pg_catalog.pg_proc p ON dep.refclassid='pg_catalog.pg_proc'::regclass AND p.oid=dep.refobjid
   LEFT JOIN pg_catalog.pg_class rc ON dep.refclassid='pg_catalog.pg_class'::regclass AND rc.oid=dep.refobjid
   JOIN pg_catalog.pg_namespace dn ON dn.oid=coalesce(p.pronamespace,rc.relnamespace)
   WHERE r.ev_class=c.oid AND dn.nspname<>'pg_catalog' AND dn.nspname<>ALL($2)
   AND (cardinality($3::text[])=0 OR coalesce(quote_ident(dn.nspname)||'.routine:'||p.proname||'('||pg_get_function_identity_arguments(p.oid)||')',quote_ident(dn.nspname)||'.relation:'||rc.relname)<>ALL($3))),
 CASE WHEN n.nspname<>$1 THEN quote_ident(n.nspname) END
FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE (n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.relation:'||c.relname=ANY($3))) AND c.relkind IN ('v','m') ORDER BY c.relname LIMIT 10001
"#,
    ),
    (
        "sequence",
        r#"
SELECT c.relname::text,jsonb_build_object('type',format_type(s.seqtypid,NULL),'start',s.seqstart,'increment',s.seqincrement,'min',s.seqmin,'max',s.seqmax,'cache',s.seqcache,'cycle',s.seqcycle,
 'owned_by',(SELECT tn.nspname||'.'||tc.relname||'.'||a.attname FROM pg_catalog.pg_depend d JOIN pg_catalog.pg_class tc ON tc.oid=d.refobjid JOIN pg_catalog.pg_namespace tn ON tn.oid=tc.relnamespace JOIN pg_catalog.pg_attribute a ON a.attrelid=tc.oid AND a.attnum=d.refobjsubid WHERE d.classid='pg_catalog.pg_class'::regclass AND d.objid=c.oid AND d.deptype IN ('a','i') LIMIT 1)),true,
 CASE WHEN n.nspname<>$1 THEN quote_ident(n.nspname) END
FROM pg_catalog.pg_sequence s JOIN pg_catalog.pg_class c ON c.oid=s.seqrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.relation:'||c.relname=ANY($3)) ORDER BY c.relname LIMIT 10001
"#,
    ),
    (
        // Extension members are owned/granted by whoever installed the
        // extension (often the bootstrap superuser); its version covers them,
        // aggregates (citext's min and max) included.
        "routine",
        r#"
SELECT p.proname||'('||pg_get_function_identity_arguments(p.oid)||')',
 jsonb_build_object('kind',p.prokind,'owner',CASE WHEN ext.oid IS NULL THEN pg_get_userbyid(p.proowner) END,'language',l.lanname,'volatility',p.provolatile,'security_definer',p.prosecdef,
 'extension',ext.extname,'definition_digest',encode(sha256(convert_to(CASE WHEN p.prokind IN ('f','p') THEN pg_get_functiondef(p.oid) ELSE p.prosrc END||coalesce(p.probin,''),'UTF8')),'hex'),
 'grants',CASE WHEN ext.oid IS NULL THEN (SELECT jsonb_agg(jsonb_build_object('grantor',pg_get_userbyid(a.grantor),'grantee',CASE WHEN a.grantee=0 THEN jsonb_build_object('public',true) ELSE to_jsonb(pg_get_userbyid(a.grantee)) END,'privilege',a.privilege_type,'grantable',a.is_grantable) ORDER BY a.grantor,a.grantee,a.privilege_type)
 FROM aclexplode(coalesce(p.proacl,acldefault('f',p.proowner))) a) END),
 (p.prokind IN ('f','p') AND l.lanname IN ('sql','plpgsql')) OR (ext.extname IS NOT NULL AND en.nspname=ANY($2)),
 CASE WHEN n.nspname<>$1 THEN quote_ident(n.nspname) END
FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace JOIN pg_catalog.pg_language l ON l.oid=p.prolang
LEFT JOIN pg_catalog.pg_depend xd ON xd.classid='pg_catalog.pg_proc'::regclass AND xd.objid=p.oid AND xd.deptype='e'
LEFT JOIN pg_catalog.pg_extension ext ON ext.oid=xd.refobjid LEFT JOIN pg_catalog.pg_namespace en ON en.oid=ext.extnamespace
-- No builtin is an outside object: skip computing their keys.
WHERE n.nspname=ANY($2) OR (n.nspname<>'pg_catalog' AND (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.routine:'||p.proname||'('||pg_get_function_identity_arguments(p.oid)||')'=ANY($3))) ORDER BY p.proname LIMIT 10001
"#,
    ),
    (
        "trigger",
        r#"
SELECT quote_ident(c.relname)||'.'||quote_ident(t.tgname),jsonb_build_object('definition',pg_get_triggerdef(t.oid,false),'enabled',t.tgenabled),
 (pn.nspname='pg_catalog' OR pn.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(pn.nspname)||'.routine:'||tp.proname||'('||pg_get_function_identity_arguments(tp.oid)||')'=ANY($3))),
 CASE WHEN n.nspname<>$1 THEN quote_ident(n.nspname) END
FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_class c ON c.oid=t.tgrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
JOIN pg_catalog.pg_proc tp ON tp.oid=t.tgfoid JOIN pg_catalog.pg_namespace pn ON pn.oid=tp.pronamespace WHERE (n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.relation:'||c.relname=ANY($3))) AND NOT t.tgisinternal ORDER BY c.relname,t.tgname LIMIT 10001
"#,
    ),
    (
        "policy",
        r#"
SELECT quote_ident(c.relname)||'.'||quote_ident(pol.polname),jsonb_build_object('command',pol.polcmd,'permissive',pol.polpermissive,'using',pg_get_expr(pol.polqual,pol.polrelid),'check',pg_get_expr(pol.polwithcheck,pol.polrelid),
 'roles',(SELECT jsonb_agg(CASE WHEN x=0 THEN 'PUBLIC' ELSE pg_get_userbyid(x) END ORDER BY x) FROM unnest(pol.polroles) x)),
 NOT EXISTS(SELECT 1 FROM pg_catalog.pg_depend dep JOIN pg_catalog.pg_proc p ON p.oid=dep.refobjid JOIN pg_catalog.pg_namespace pn ON pn.oid=p.pronamespace WHERE dep.classid='pg_catalog.pg_policy'::regclass AND dep.objid=pol.oid AND dep.refclassid='pg_catalog.pg_proc'::regclass AND pn.nspname<>'pg_catalog' AND pn.nspname<>ALL($2)
 AND (cardinality($3::text[])=0 OR quote_ident(pn.nspname)||'.routine:'||p.proname||'('||pg_get_function_identity_arguments(p.oid)||')'<>ALL($3))),
 CASE WHEN n.nspname<>$1 THEN quote_ident(n.nspname) END
FROM pg_catalog.pg_policy pol JOIN pg_catalog.pg_class c ON c.oid=pol.polrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.relation:'||c.relname=ANY($3)) ORDER BY c.relname,pol.polname LIMIT 10001
"#,
    ),
    (
        // Enums (by their labels in order) and domains (base type, NOT NULL,
        // default, collation and constraints) are compared like tables. Members
        // of an extension installed in the scope follow its version, as its
        // routines do. Range and other base types stay uncovered.
        "type",
        r#"
SELECT t.typname::text,jsonb_build_object('kind',t.typtype,'owner',CASE WHEN ext.oid IS NULL THEN pg_get_userbyid(t.typowner) END,'definition',format_type(t.oid,NULL),
 'extension',ext.extname,'comment',obj_description(t.oid,'pg_type'),
 'grants',CASE WHEN ext.oid IS NULL THEN (SELECT jsonb_agg(jsonb_build_object('grantor',pg_get_userbyid(a.grantor),'grantee',CASE WHEN a.grantee=0 THEN jsonb_build_object('public',true) ELSE to_jsonb(pg_get_userbyid(a.grantee)) END,'privilege',a.privilege_type,'grantable',a.is_grantable) ORDER BY a.grantor,a.grantee,a.privilege_type)
 FROM aclexplode(coalesce(t.typacl,acldefault('T',t.typowner))) a) END,
 'labels',(SELECT jsonb_agg(e.enumlabel ORDER BY e.enumsortorder) FROM pg_catalog.pg_enum e WHERE e.enumtypid=t.oid),
 'base_type',CASE WHEN t.typtype='d' THEN format_type(t.typbasetype,t.typtypmod) END,'not_null',t.typnotnull,'default',pg_get_expr(t.typdefaultbin,0),
 'collation',(SELECT quote_ident(cn.nspname)||'.'||quote_ident(co.collname) FROM pg_catalog.pg_collation co JOIN pg_catalog.pg_namespace cn ON cn.oid=co.collnamespace WHERE co.oid=t.typcollation),
 'constraints',(SELECT jsonb_agg(jsonb_build_object('name',k.conname,'kind',k.contype,'validated',k.convalidated,
   'definition_digest',encode(sha256(convert_to(pg_get_constraintdef(k.oid,false),'UTF8')),'hex')) ORDER BY k.conname) FROM pg_catalog.pg_constraint k WHERE k.contypid=t.oid)),
 CASE WHEN ext.oid IS NOT NULL THEN en.nspname=ANY($2) WHEN t.typtype='e' THEN true
 WHEN t.typtype='d' THEN (btn.nspname='pg_catalog' OR btn.nspname=ANY($2) OR (cardinality($3::text[])>0 AND (SELECT quote_ident(rn.nspname)||CASE WHEN rt.typrelid<>0 THEN '.relation:'||rc.relname ELSE '.type:'||rt.typname END
   FROM pg_catalog.pg_type rt JOIN pg_catalog.pg_namespace rn ON rn.oid=rt.typnamespace LEFT JOIN pg_catalog.pg_class rc ON rc.oid=rt.typrelid
   WHERE rt.oid=CASE WHEN bt.typelem<>0 AND bt.typlen=-1 THEN bt.typelem ELSE bt.oid END)=ANY($3)))
  AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_collation co JOIN pg_catalog.pg_namespace cn ON cn.oid=co.collnamespace WHERE co.oid=t.typcollation AND cn.nspname<>'pg_catalog')
  -- What the default and the constraints call.
  AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_depend dep JOIN pg_catalog.pg_proc p ON p.oid=dep.refobjid JOIN pg_catalog.pg_namespace pn ON pn.oid=p.pronamespace
   WHERE ((dep.classid='pg_catalog.pg_type'::regclass AND dep.objid=t.oid) OR (dep.classid='pg_catalog.pg_constraint'::regclass AND dep.objid IN (SELECT k.oid FROM pg_catalog.pg_constraint k WHERE k.contypid=t.oid)))
   AND dep.refclassid='pg_catalog.pg_proc'::regclass AND pn.nspname<>'pg_catalog' AND pn.nspname<>ALL($2)
   AND (cardinality($3::text[])=0 OR quote_ident(pn.nspname)||'.routine:'||p.proname||'('||pg_get_function_identity_arguments(p.oid)||')'<>ALL($3)))
 ELSE false END,
 CASE WHEN n.nspname<>$1 THEN quote_ident(n.nspname) END
FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace
LEFT JOIN pg_catalog.pg_type bt ON bt.oid=t.typbasetype LEFT JOIN pg_catalog.pg_namespace btn ON btn.oid=bt.typnamespace
LEFT JOIN pg_catalog.pg_depend xd ON xd.classid='pg_catalog.pg_type'::regclass AND xd.objid=t.oid AND xd.deptype='e'
LEFT JOIN pg_catalog.pg_extension ext ON ext.oid=xd.refobjid LEFT JOIN pg_catalog.pg_namespace en ON en.oid=ext.extnamespace
WHERE (n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.type:'||t.typname=ANY($3))) AND t.typrelid=0 AND t.typelem=0 ORDER BY t.typname LIMIT 10001
"#,
    ),
    (
        "extension",
        r#"
SELECT e.extname::text,jsonb_build_object('version',e.extversion,'schema',n.nspname,'external',n.nspname<>ALL($2)),
 n.nspname=ANY($2) OR (e.extname=ANY($4) AND n.nspname NOT IN ('_ntnt','information_schema') AND NOT starts_with(n.nspname,'pg_')),NULL::text
FROM pg_catalog.pg_extension e JOIN pg_catalog.pg_namespace n ON n.oid=e.extnamespace WHERE e.extname<>'plpgsql' AND $1::text IS NOT NULL AND $3::text[] IS NOT NULL ORDER BY e.extname LIMIT 10001
"#,
    ),
    (
        // Members of an extension installed in the scope follow its version,
        // as its routines and types do; owner and grants are the installer's.
        // An operator is keyed with its argument types, as a routine is, and
        // an operator class or family with its access method.
        "other",
        r#"
SELECT o.name,CASE WHEN ext.oid IS NULL THEN o.value ELSE o.value||jsonb_build_object('owner',NULL,'extension',ext.extname) END,
 CASE WHEN ext.oid IS NULL THEN o.supported ELSE en.nspname=ANY($2) END,CASE WHEN o.nspname<>$1 THEN quote_ident(o.nspname) END
FROM (
SELECT 'operator.'||o.oprname||'('||CASE WHEN o.oprleft=0 THEN 'NONE' ELSE format_type(o.oprleft,NULL) END||','||CASE WHEN o.oprright=0 THEN 'NONE' ELSE format_type(o.oprright,NULL) END||')' AS name,
 jsonb_build_object('kind','operator','owner',pg_get_userbyid(o.oprowner),'result',format_type(o.oprresult,NULL),'function',o.oprcode::oid::regprocedure::text,
 'commutator',nullif(o.oprcom,0)::regoperator::text,'negator',nullif(o.oprnegate,0)::regoperator::text,'restrict',nullif(o.oprrest::oid,0)::regprocedure::text,
 'join',nullif(o.oprjoin::oid,0)::regprocedure::text,'hashes',o.oprcanhash,'merges',o.oprcanmerge,'comment',obj_description(o.oid,'pg_operator')) AS value,
 -- An operator is its function, as covered as any routine call.
 NOT EXISTS(SELECT 1 FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace pn ON pn.oid=p.pronamespace WHERE p.oid IN (o.oprcode,o.oprrest,o.oprjoin) AND pn.nspname<>'pg_catalog' AND pn.nspname<>ALL($2)
 AND (cardinality($3::text[])=0 OR quote_ident(pn.nspname)||'.routine:'||p.proname||'('||pg_get_function_identity_arguments(p.oid)||')'<>ALL($3))) AS supported,
 n.nspname,'pg_catalog.pg_operator'::regclass::oid AS classid,o.oid AS objid FROM pg_catalog.pg_operator o JOIN pg_catalog.pg_namespace n ON n.oid=o.oprnamespace
 WHERE n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.other:operator.'||o.oprname||'('||CASE WHEN o.oprleft=0 THEN 'NONE' ELSE format_type(o.oprleft,NULL) END||','||CASE WHEN o.oprright=0 THEN 'NONE' ELSE format_type(o.oprright,NULL) END||')'=ANY($3))
UNION ALL SELECT 'collation.'||c.collname,jsonb_build_object('kind','collation','version',c.collversion),false,n.nspname,'pg_catalog.pg_collation'::regclass::oid,c.oid FROM pg_catalog.pg_collation c JOIN pg_catalog.pg_namespace n ON n.oid=c.collnamespace WHERE n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.other:collation.'||c.collname=ANY($3))
UNION ALL SELECT 'conversion.'||c.conname,jsonb_build_object('kind','conversion'),false,n.nspname,'pg_catalog.pg_conversion'::regclass::oid,c.oid FROM pg_catalog.pg_conversion c JOIN pg_catalog.pg_namespace n ON n.oid=c.connamespace WHERE n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.other:conversion.'||c.conname=ANY($3))
UNION ALL SELECT 'default_acl.'||pg_get_userbyid(d.defaclrole)||'.'||d.defaclobjtype::text,jsonb_build_object('kind','default_acl','external',d.defaclnamespace=0),false,n.nspname,NULL,NULL FROM pg_catalog.pg_default_acl d LEFT JOIN pg_catalog.pg_namespace n ON n.oid=d.defaclnamespace WHERE n.nspname=ANY($2) OR d.defaclnamespace=0
UNION ALL SELECT 'rule.'||quote_ident(c.relname)||'.'||quote_ident(r.rulename),jsonb_build_object('kind','rule','definition',pg_get_ruledef(r.oid,false)),false,n.nspname,NULL,NULL FROM pg_catalog.pg_rewrite r JOIN pg_catalog.pg_class c ON c.oid=r.ev_class JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE (n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.relation:'||c.relname=ANY($3))) AND r.rulename<>'_RETURN'
UNION ALL SELECT 'operator_class.'||quote_ident(o.opcname)||' USING '||quote_ident(am.amname),jsonb_build_object('kind','operator_class'),false,n.nspname,'pg_catalog.pg_opclass'::regclass::oid,o.oid FROM pg_catalog.pg_opclass o JOIN pg_catalog.pg_am am ON am.oid=o.opcmethod JOIN pg_catalog.pg_namespace n ON n.oid=o.opcnamespace WHERE n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.other:operator_class.'||quote_ident(o.opcname)||' USING '||quote_ident(am.amname)=ANY($3))
UNION ALL SELECT 'operator_family.'||quote_ident(o.opfname)||' USING '||quote_ident(am.amname),jsonb_build_object('kind','operator_family'),false,n.nspname,'pg_catalog.pg_opfamily'::regclass::oid,o.oid FROM pg_catalog.pg_opfamily o JOIN pg_catalog.pg_am am ON am.oid=o.opfmethod JOIN pg_catalog.pg_namespace n ON n.oid=o.opfnamespace WHERE n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.other:operator_family.'||quote_ident(o.opfname)||' USING '||quote_ident(am.amname)=ANY($3))
UNION ALL SELECT 'text_search_parser.'||quote_ident(p.prsname),jsonb_build_object('kind','text_search'),false,n.nspname,'pg_catalog.pg_ts_parser'::regclass::oid,p.oid FROM pg_catalog.pg_ts_parser p JOIN pg_catalog.pg_namespace n ON n.oid=p.prsnamespace WHERE n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.other:text_search_parser.'||quote_ident(p.prsname)=ANY($3))
UNION ALL SELECT 'text_search_template.'||quote_ident(t.tmplname),jsonb_build_object('kind','text_search'),false,n.nspname,'pg_catalog.pg_ts_template'::regclass::oid,t.oid FROM pg_catalog.pg_ts_template t JOIN pg_catalog.pg_namespace n ON n.oid=t.tmplnamespace WHERE n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.other:text_search_template.'||quote_ident(t.tmplname)=ANY($3))
UNION ALL SELECT 'security_label.'||pg_describe_object(s.classoid,s.objoid,s.objsubid)||'.'||s.provider,jsonb_build_object('kind','security_label','definition',s.label,'external',true),false,NULL,NULL,NULL FROM pg_catalog.pg_seclabel s
UNION ALL SELECT 'shared_security_label.'||pg_describe_object(s.classoid,s.objoid,0)||'.'||s.provider,jsonb_build_object('kind','security_label','definition',s.label,'external',true),false,NULL,NULL,NULL FROM pg_catalog.pg_shseclabel s
UNION ALL SELECT 'event_trigger.'||quote_ident(e.evtname),jsonb_build_object('kind','event_trigger','enabled',e.evtenabled,'external',true),false,NULL,NULL,NULL FROM pg_catalog.pg_event_trigger e
UNION ALL SELECT 'statistics.'||s.stxname,jsonb_build_object('kind','statistics'),false,n.nspname,'pg_catalog.pg_statistic_ext'::regclass::oid,s.oid FROM pg_catalog.pg_statistic_ext s JOIN pg_catalog.pg_namespace n ON n.oid=s.stxnamespace WHERE n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.other:statistics.'||s.stxname=ANY($3))
UNION ALL SELECT 'text_search_config.'||c.cfgname,jsonb_build_object('kind','text_search'),false,n.nspname,'pg_catalog.pg_ts_config'::regclass::oid,c.oid FROM pg_catalog.pg_ts_config c JOIN pg_catalog.pg_namespace n ON n.oid=c.cfgnamespace WHERE n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.other:text_search_config.'||c.cfgname=ANY($3))
UNION ALL SELECT 'text_search_dictionary.'||d.dictname,jsonb_build_object('kind','text_search'),false,n.nspname,'pg_catalog.pg_ts_dict'::regclass::oid,d.oid FROM pg_catalog.pg_ts_dict d JOIN pg_catalog.pg_namespace n ON n.oid=d.dictnamespace WHERE n.nspname=ANY($2) OR (cardinality($3::text[])>0 AND quote_ident(n.nspname)||'.other:text_search_dictionary.'||d.dictname=ANY($3))
) o LEFT JOIN pg_catalog.pg_depend xd ON xd.classid=o.classid AND xd.objid=o.objid AND xd.deptype='e'
LEFT JOIN pg_catalog.pg_extension ext ON ext.oid=xd.refobjid LEFT JOIN pg_catalog.pg_namespace en ON en.oid=ext.extnamespace
WHERE $1::text IS NOT NULL
"#,
    ),
];

/// The `QUERIES` that take `$4`, the declared extensions.
pub(super) const WITH_DECLARED_EXTENSIONS: &[&str] = &["relation", "extension"];

/// What the declared extensions (`$1`, text[]) own, as a `WITH RECURSIVE`
/// element: `owned(classid, objid)`, their members (pg_depend `e`) and,
/// transitively, what depends on one of those internally or automatically
/// (`i`, `a`), such as array and row types, indexes and TOAST tables.
/// Superuser-owned, it is what `prepare_scratch` accepts in a scratch
/// database's `public` and in the declared extensions' schemas, and what
/// `OUTSIDE_OBJECTS` does not take for a replayed history's work (#430).
pub(super) const DECLARED_OWNED: &str = declared_owned!("$1");

/// What a replayed history has created outside the managed scope in scratch,
/// by the catalog key `QUERIES` give it, for their `$3`. Scratch holds nothing
/// else outside the scope, so this is exactly the history's work there:
/// - each schema it created (`$1` is the managed scope; `$2` the schemas
///   scratch was prepared with, such as `public`, which it did not create);
/// - in any other schema but PostgreSQL's, `_ntnt` and NTNT's replay marker
///   `_ntnt_replay`: its relations other than indexes, routines, types and
///   other namespace objects.
///
/// A relation's indexes, constraints, triggers, policies and rules follow
/// it, and an extension's members follow the extension, which `QUERIES`
/// inspect database-wide, so neither is listed. Default privileges for a
/// schema are not objects and are not listed either.
///
/// What the declared extensions (`$3`) own (`DECLARED_OWNED`) was installed
/// before replay, as on the target, so it is not listed either: their
/// members, and what is not a member but depends on one, such as the
/// relations of tablefunc's and dblink's composite types (#430).
pub(super) const OUTSIDE_OBJECTS: &str = concat!(
    r#"
WITH RECURSIVE "#,
    declared_owned!("$3"),
    r#",
o AS (SELECT n.oid,n.nspname FROM pg_catalog.pg_namespace n
 WHERE n.nspname<>ALL($1) AND n.nspname NOT IN ('information_schema','_ntnt','_ntnt_replay') AND NOT starts_with(n.nspname,'pg_')),
e AS (SELECT d.classid,d.objid FROM pg_catalog.pg_depend d WHERE d.deptype='e' UNION SELECT w.classid,w.objid FROM owned w)
SELECT 'schema:'||o.nspname FROM o WHERE o.nspname<>ALL($2)
UNION ALL SELECT quote_ident(o.nspname)||'.relation:'||c.relname FROM pg_catalog.pg_class c JOIN o ON o.oid=c.relnamespace
 WHERE c.relkind NOT IN ('i','I') AND NOT EXISTS(SELECT 1 FROM e WHERE e.classid='pg_catalog.pg_class'::regclass AND e.objid=c.oid)
UNION ALL SELECT quote_ident(o.nspname)||'.routine:'||p.proname||'('||pg_get_function_identity_arguments(p.oid)||')' FROM pg_catalog.pg_proc p JOIN o ON o.oid=p.pronamespace
 WHERE NOT EXISTS(SELECT 1 FROM e WHERE e.classid='pg_catalog.pg_proc'::regclass AND e.objid=p.oid)
UNION ALL SELECT quote_ident(o.nspname)||'.type:'||t.typname FROM pg_catalog.pg_type t JOIN o ON o.oid=t.typnamespace
 WHERE t.typrelid=0 AND t.typelem=0 AND NOT EXISTS(SELECT 1 FROM e WHERE e.classid='pg_catalog.pg_type'::regclass AND e.objid=t.oid)
UNION ALL SELECT quote_ident(o.nspname)||'.other:operator.'||x.oprname||'('||CASE WHEN x.oprleft=0 THEN 'NONE' ELSE format_type(x.oprleft,NULL) END||','||CASE WHEN x.oprright=0 THEN 'NONE' ELSE format_type(x.oprright,NULL) END||')' FROM pg_catalog.pg_operator x JOIN o ON o.oid=x.oprnamespace
 WHERE NOT EXISTS(SELECT 1 FROM e WHERE e.classid='pg_catalog.pg_operator'::regclass AND e.objid=x.oid)
UNION ALL SELECT quote_ident(o.nspname)||'.other:collation.'||x.collname FROM pg_catalog.pg_collation x JOIN o ON o.oid=x.collnamespace
 WHERE NOT EXISTS(SELECT 1 FROM e WHERE e.classid='pg_catalog.pg_collation'::regclass AND e.objid=x.oid)
UNION ALL SELECT quote_ident(o.nspname)||'.other:conversion.'||x.conname FROM pg_catalog.pg_conversion x JOIN o ON o.oid=x.connamespace
 WHERE NOT EXISTS(SELECT 1 FROM e WHERE e.classid='pg_catalog.pg_conversion'::regclass AND e.objid=x.oid)
UNION ALL SELECT quote_ident(o.nspname)||'.other:operator_class.'||quote_ident(x.opcname)||' USING '||quote_ident(am.amname) FROM pg_catalog.pg_opclass x JOIN o ON o.oid=x.opcnamespace JOIN pg_catalog.pg_am am ON am.oid=x.opcmethod
 WHERE NOT EXISTS(SELECT 1 FROM e WHERE e.classid='pg_catalog.pg_opclass'::regclass AND e.objid=x.oid)
UNION ALL SELECT quote_ident(o.nspname)||'.other:operator_family.'||quote_ident(x.opfname)||' USING '||quote_ident(am.amname) FROM pg_catalog.pg_opfamily x JOIN o ON o.oid=x.opfnamespace JOIN pg_catalog.pg_am am ON am.oid=x.opfmethod
 WHERE NOT EXISTS(SELECT 1 FROM e WHERE e.classid='pg_catalog.pg_opfamily'::regclass AND e.objid=x.oid)
UNION ALL SELECT quote_ident(o.nspname)||'.other:statistics.'||x.stxname FROM pg_catalog.pg_statistic_ext x JOIN o ON o.oid=x.stxnamespace
UNION ALL SELECT quote_ident(o.nspname)||'.other:text_search_parser.'||quote_ident(x.prsname) FROM pg_catalog.pg_ts_parser x JOIN o ON o.oid=x.prsnamespace
 WHERE NOT EXISTS(SELECT 1 FROM e WHERE e.classid='pg_catalog.pg_ts_parser'::regclass AND e.objid=x.oid)
UNION ALL SELECT quote_ident(o.nspname)||'.other:text_search_template.'||quote_ident(x.tmplname) FROM pg_catalog.pg_ts_template x JOIN o ON o.oid=x.tmplnamespace
 WHERE NOT EXISTS(SELECT 1 FROM e WHERE e.classid='pg_catalog.pg_ts_template'::regclass AND e.objid=x.oid)
UNION ALL SELECT quote_ident(o.nspname)||'.other:text_search_config.'||x.cfgname FROM pg_catalog.pg_ts_config x JOIN o ON o.oid=x.cfgnamespace
 WHERE NOT EXISTS(SELECT 1 FROM e WHERE e.classid='pg_catalog.pg_ts_config'::regclass AND e.objid=x.oid)
UNION ALL SELECT quote_ident(o.nspname)||'.other:text_search_dictionary.'||x.dictname FROM pg_catalog.pg_ts_dict x JOIN o ON o.oid=x.dictnamespace
 WHERE NOT EXISTS(SELECT 1 FROM e WHERE e.classid='pg_catalog.pg_ts_dict'::regclass AND e.objid=x.oid)
ORDER BY 1 LIMIT 10001
"#,
);
