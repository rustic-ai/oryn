# Architecture: Pushdown Hydration

**Document:** Technical Design Proposal  
**Date:** 2026-01-27  
**Status:** Draft  
**Related:** `ARCH_EDGE_PROPERTY_HYDRATION.md`

---

## Executive Summary

This document proposes **pushdown hydration** as the architecturally correct solution to the edge property hydration problem. Instead of loading properties lazily at function call time, we analyze property requirements during query planning and push them down to the initial graph scan.

**Key Insight:** If we know what properties a query needs *before* execution, we can load them in a single storage scan rather than multiple lazy fetches.

---

## Table of Contents

1. [Problem Recap](#problem-recap)
2. [What is Pushdown Hydration?](#what-is-pushdown-hydration)
3. [Conceptual Overview](#conceptual-overview)
4. [Phase 1: Property Requirement Analysis](#phase-1-property-requirement-analysis)
5. [Phase 2: Requirement Propagation](#phase-2-requirement-propagation)
6. [Phase 3: Scan Modification](#phase-3-scan-modification)
7. [Phase 4: Hydrated Execution](#phase-4-hydrated-execution)
8. [Special Cases](#special-cases)
9. [Query Rewriting vs Pushdown](#query-rewriting-vs-pushdown)
10. [Implementation Roadmap](#implementation-roadmap)
11. [Performance Analysis](#performance-analysis)

---

## Problem Recap

### Current Flow (Lazy Hydration)

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Query: MATCH (p)-[e:WORKS_AT]->(c)                                      │
│        WHERE validAt(e, 'start', 'end', $ts)                            │
│        RETURN e.role                                                    │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ Step 1: MATCH scans adjacency dataset                                   │
│         Returns: e = {_eid, _src, _dst, _type}     ← IDs only!          │
│         Storage scans: 1                                                │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ Step 2: WHERE evaluates validAt(e, ...)                                 │
│         Function receives: {_eid: 0, _src: 0, _dst: 1, _type: 1}        │
│         ❌ FAILS - no 'start' or 'end' properties!                      │
└─────────────────────────────────────────────────────────────────────────┘
```

### Eager Hydration (Current Proposed Fix)

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Step 2: WHERE evaluates validAt(e, ...)                                 │
│         → Hydrate e before function call                                │
│         → Storage scan #2: load ALL properties for e                    │
│         → Now e = {_eid, _src, _dst, _type, start, end, role, ...}      │
│         ✅ Works, but extra storage scan                                │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ Step 3: RETURN e.role                                                   │
│         → Already hydrated, cache hit                                   │
│         Storage scans: 2 total                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

**Problem:** Two separate storage accesses. For N edges, this is O(2N) scans.

---

## What is Pushdown Hydration?

**Pushdown hydration** means determining property requirements at *plan time* and including them in the *initial scan*, so entities are fully hydrated from the start.

### Pushdown Flow

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Query: MATCH (p)-[e:WORKS_AT]->(c)                                      │
│        WHERE validAt(e, 'start', 'end', $ts)                            │
│        RETURN e.role                                                    │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ PLANNING PHASE                                                          │
│                                                                         │
│ Analyze query → e needs: {start, end, role}                             │
│ Push requirements to MATCH clause                                       │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ EXECUTION PHASE                                                         │
│                                                                         │
│ Step 1: MATCH scans adjacency + edge dataset (joined)                   │
│         Returns: e = {_eid, _src, _dst, _type, start, end, role}        │
│         Storage scans: 1 (single joined scan)                           │
│                                                                         │
│ Step 2: WHERE evaluates validAt(e, ...)                                 │
│         ✅ Properties already present                                   │
│                                                                         │
│ Step 3: RETURN e.role                                                   │
│         ✅ Already loaded                                               │
└─────────────────────────────────────────────────────────────────────────┘
```

**Result:** Single storage scan regardless of how properties are used.

---

## Conceptual Overview

Pushdown hydration has four phases:

```
┌──────────────────────────────────────────────────────────────────────────┐
│                        PUSHDOWN HYDRATION PIPELINE                       │
└──────────────────────────────────────────────────────────────────────────┘

  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐
  │   PHASE 1   │    │   PHASE 2   │    │   PHASE 3   │    │   PHASE 4   │
  │             │    │             │    │             │    │             │
  │  Property   │───▶│ Requirement │───▶│    Scan     │───▶│  Hydrated   │
  │  Requirement│    │ Propagation │    │ Modification│    │  Execution  │
  │  Analysis   │    │             │    │             │    │             │
  └─────────────┘    └─────────────┘    └─────────────┘    └─────────────┘
        │                  │                  │                  │
        ▼                  ▼                  ▼                  ▼
   "What props      "Push reqs to       "Modify scan      "Execute with
    are needed?"     source nodes"       to include        pre-loaded
                                         properties"       properties"
```

---

## Phase 1: Property Requirement Analysis

### Goal

Walk the query AST and determine, for each variable, which properties are accessed.

### Input

```cypher
MATCH (p:Person)-[e:EMPLOYED_BY]->(c:Company)
WHERE uni.temporal.validAt(e, 'valid_from', 'valid_to', datetime('2021-06-15'))
  AND p.age > 30
RETURN p.name, e.role, c.name
```

### Output: Property Requirement Map

```rust
PropertyRequirements {
    "p": { "age", "name" },
    "e": { "valid_from", "valid_to", "role" },
    "c": { "name" }
}
```

### Algorithm

```rust
struct PropertyAnalyzer {
    requirements: HashMap<String, HashSet<String>>,
}

impl PropertyAnalyzer {
    fn analyze(&mut self, expr: &Expr) {
        match expr {
            // Direct property access: p.name, e.role
            Expr::Property(var, prop_name) => {
                let var_name = var.as_variable_name();
                self.requirements
                    .entry(var_name)
                    .or_default()
                    .insert(prop_name.clone());
            }

            // Function calls: validAt(e, 'valid_from', 'valid_to', ...)
            Expr::FunctionCall { name, args } => {
                self.analyze_function_call(name, args);
            }

            // Recurse into subexpressions
            Expr::BinaryOp { left, right, .. } => {
                self.analyze(left);
                self.analyze(right);
            }

            Expr::UnaryOp { operand, .. } => {
                self.analyze(operand);
            }

            // ... handle other expression types
        }
    }

    fn analyze_function_call(&mut self, name: &str, args: &[Expr]) {
        match name {
            // Known temporal functions - extract property names from args
            "uni.temporal.validAt" => {
                // validAt(entity, start_prop, end_prop, timestamp)
                if let Some(var_name) = args.get(0).and_then(|e| e.as_variable_name()) {
                    if let Some(start) = args.get(1).and_then(|e| e.as_string_literal()) {
                        self.requirements.entry(var_name.clone()).or_default().insert(start);
                    }
                    if let Some(end) = args.get(2).and_then(|e| e.as_string_literal()) {
                        self.requirements.entry(var_name.clone()).or_default().insert(end);
                    }
                }
            }

            // Unknown functions - if entity passed, require ALL properties (conservative)
            _ => {
                for arg in args {
                    if let Some(var_name) = arg.as_variable_name() {
                        self.requirements
                            .entry(var_name)
                            .or_default()
                            .insert("*".to_string()); // Wildcard = all properties
                    }
                }
            }
        }

        // Recurse into function arguments
        for arg in args {
            self.analyze(arg);
        }
    }
}
```

### Function Registry for Property Extraction

To avoid requiring `*` (all properties) for every unknown function, maintain a registry:

```rust
struct FunctionPropertySpec {
    /// Which argument positions contain entity references
    entity_args: Vec<usize>,
    
    /// Which argument positions contain property name strings
    /// Maps: argument index → entity argument it references
    property_name_args: Vec<(usize, usize)>,
    
    /// If true, function needs ALL properties of entity args
    needs_full_entity: bool,
}

lazy_static! {
    static ref FUNCTION_SPECS: HashMap<&'static str, FunctionPropertySpec> = {
        let mut m = HashMap::new();
        
        // validAt(entity, start_prop, end_prop, timestamp)
        m.insert("uni.temporal.validAt", FunctionPropertySpec {
            entity_args: vec![0],
            property_name_args: vec![(1, 0), (2, 0)], // args 1,2 are prop names for arg 0
            needs_full_entity: false,
        });

        // overlaps(entity, start_prop, end_prop, range_start, range_end)
        m.insert("uni.temporal.overlaps", FunctionPropertySpec {
            entity_args: vec![0],
            property_name_args: vec![(1, 0), (2, 0)],
            needs_full_entity: false,
        });

        // keys(entity) - needs all properties to return keys
        m.insert("keys", FunctionPropertySpec {
            entity_args: vec![0],
            property_name_args: vec![],
            needs_full_entity: true,
        });

        // properties(entity) - needs all properties
        m.insert("properties", FunctionPropertySpec {
            entity_args: vec![0],
            property_name_args: vec![],
            needs_full_entity: true,
        });

        m
    };
}
```

---

## Phase 2: Requirement Propagation

### Goal

Attach property requirements to the appropriate scan operators in the query plan.

### Query Plan Structure

```
┌─────────────────────────────────────────────────────────────────────────┐
│                          LOGICAL QUERY PLAN                             │
└─────────────────────────────────────────────────────────────────────────┘

                              ┌──────────┐
                              │  RETURN  │
                              │ p.name,  │
                              │ e.role,  │
                              │ c.name   │
                              └────┬─────┘
                                   │
                              ┌────┴─────┐
                              │  FILTER  │
                              │validAt() │
                              │ p.age>30 │
                              └────┬─────┘
                                   │
                         ┌─────────┴─────────┐
                         │      EXPAND       │
                         │ (p)-[e:EMP]->(c)  │
                         └─────────┬─────────┘
                                   │
                    ┌──────────────┼──────────────┐
                    │              │              │
              ┌─────┴─────┐ ┌─────┴─────┐ ┌─────┴─────┐
              │   SCAN    │ │   SCAN    │ │   SCAN    │
              │  p:Person │ │ e:EMP_BY  │ │ c:Company │
              └───────────┘ └───────────┘ └───────────┘
```

### Propagation Algorithm

Requirements flow **downward** from usage sites to scan nodes:

```rust
struct RequirementPropagator {
    requirements: HashMap<String, HashSet<String>>,
}

impl RequirementPropagator {
    fn propagate(&self, plan: &mut LogicalPlan) {
        self.propagate_node(plan);
    }

    fn propagate_node(&self, node: &mut LogicalPlan) {
        match node {
            // Scan nodes receive requirements for their variable
            LogicalPlan::NodeScan { variable, required_properties, .. } => {
                if let Some(reqs) = self.requirements.get(variable) {
                    *required_properties = reqs.clone();
                }
            }

            LogicalPlan::EdgeScan { variable, required_properties, .. } => {
                if let Some(reqs) = self.requirements.get(variable) {
                    *required_properties = reqs.clone();
                }
            }

            LogicalPlan::Expand { edge_variable, required_edge_properties, child, .. } => {
                if let Some(reqs) = self.requirements.get(edge_variable) {
                    *required_edge_properties = reqs.clone();
                }
                self.propagate_node(child);
            }

            // Recurse through other nodes
            LogicalPlan::Filter { child, .. } => self.propagate_node(child),
            LogicalPlan::Project { child, .. } => self.propagate_node(child),
            LogicalPlan::Join { left, right, .. } => {
                self.propagate_node(left);
                self.propagate_node(right);
            }
            // ... other plan nodes
        }
    }
}
```

### Annotated Plan After Propagation

```
                              ┌──────────┐
                              │  RETURN  │
                              └────┬─────┘
                                   │
                              ┌────┴─────┐
                              │  FILTER  │
                              └────┬─────┘
                                   │
                         ┌─────────┴─────────┐
                         │      EXPAND       │
                         │ e.required_props: │
                         │ {valid_from,      │
                         │  valid_to, role}  │
                         └─────────┬─────────┘
                                   │
                    ┌──────────────┼──────────────┐
                    │              │              │
              ┌─────┴─────┐       ...       ┌─────┴─────┐
              │   SCAN    │                 │   SCAN    │
              │  p:Person │                 │ c:Company │
              │ req: {age,│                 │ req:{name}│
              │      name}│                 └───────────┘
              └───────────┘
```

---

## Phase 3: Scan Modification

### Goal

Modify scan operators to fetch required properties alongside topology.

### Current Scan (Adjacency Only)

```rust
// Current implementation
async fn scan_edges(
    &self,
    edge_type: EdgeTypeId,
    direction: Direction,
) -> Result<Vec<EdgeRef>> {
    // Only reads from adjacency dataset
    let adjacency = self.adjacency_dataset.scan(edge_type, direction).await?;
    
    adjacency.map(|row| EdgeRef {
        eid: row.eid,
        src: row.src,
        dst: row.dst,
        edge_type: row.edge_type,
        // No properties!
    }).collect()
}
```

### Modified Scan (With Property Hydration)

```rust
// New implementation with property requirements
async fn scan_edges_hydrated(
    &self,
    edge_type: EdgeTypeId,
    direction: Direction,
    required_properties: &HashSet<String>,
) -> Result<Vec<HydratedEdge>> {
    
    // Step 1: Scan adjacency for topology
    let adjacency = self.adjacency_dataset.scan(edge_type, direction).await?;
    let eids: Vec<Eid> = adjacency.iter().map(|r| r.eid).collect();
    
    // Step 2: Batch fetch properties from edge dataset
    let properties = if required_properties.contains("*") {
        // Wildcard: fetch all properties
        self.edge_dataset.batch_get_all_properties(&eids).await?
    } else if !required_properties.is_empty() {
        // Specific properties: project only what's needed
        self.edge_dataset
            .batch_get_properties(&eids, required_properties)
            .await?
    } else {
        // No properties needed
        HashMap::new()
    };
    
    // Step 3: Join topology with properties
    adjacency.into_iter().map(|row| {
        let mut props = properties.get(&row.eid).cloned().unwrap_or_default();
        
        // Always include system properties
        props.insert("_eid".to_string(), Value::from(row.eid));
        props.insert("_src".to_string(), Value::from(row.src));
        props.insert("_dst".to_string(), Value::from(row.dst));
        props.insert("_type".to_string(), Value::from(row.edge_type));
        
        HydratedEdge { properties: props }
    }).collect()
}
```

### Optimized: Single Joined Scan

Even better — if both datasets are in Lance, do a single joined scan:

```rust
async fn scan_edges_optimized(
    &self,
    edge_type: EdgeTypeId,
    required_properties: &HashSet<String>,
) -> Result<RecordBatch> {
    
    // Build projection list
    let mut projection = vec!["_eid", "_src", "_dst", "_type"];
    
    if required_properties.contains("*") {
        projection.push("*"); // All columns
    } else {
        projection.extend(required_properties.iter().map(|s| s.as_str()));
    }
    
    // Single scan with projection pushdown to Lance
    self.edge_dataset
        .scan()
        .filter(col("_type").eq(lit(edge_type)))
        .project(&projection)?
        .execute()
        .await
}
```

---

## Phase 4: Hydrated Execution

### Goal

Execute the query with pre-hydrated entities — no lazy loading needed.

### Row Format

```rust
// Before: Row contains entity references with IDs only
struct Row {
    bindings: HashMap<String, Value>,
    // e.g., "e" → Value::Object({_eid: 0, _src: 0, _dst: 1, _type: 1})
}

// After: Row contains hydrated entities
struct Row {
    bindings: HashMap<String, Value>,
    // e.g., "e" → Value::Object({
    //     _eid: 0, _src: 0, _dst: 1, _type: 1,
    //     valid_from: "2020-01-01", valid_to: "2022-12-31", role: "Engineer"
    // })
}
```

### Execution Changes

With hydrated rows, most execution code remains unchanged:

```rust
// Property access - works as before (but now always hits)
Expr::Property(var, prop_name) => {
    let entity = row.get(var)?;
    if let Value::Object(map) = entity {
        // Property is already in the map!
        return Ok(map.get(prop_name).cloned().unwrap_or(Value::Null));
    }
}

// Function calls - no hydration needed!
Expr::FunctionCall { name, args } => {
    let evaluated_args: Vec<Value> = args
        .iter()
        .map(|arg| evaluate_expr(arg, row))  // Already hydrated
        .collect();
    
    eval_scalar_function(name, &evaluated_args)
}
```

### Fallback for Missed Properties

If a property wasn't in the requirements (edge case: dynamic property access), fall back to lazy loading:

```rust
Expr::Property(var, prop_name) => {
    let entity = row.get(var)?;
    if let Value::Object(map) = entity {
        // Try hydrated value first
        if let Some(val) = map.get(prop_name) {
            return Ok(val.clone());
        }
        
        // Fallback: lazy load (should be rare)
        if let Some(eid) = map.get("_eid").and_then(|v| v.as_u64()) {
            return prop_manager.get_edge_prop(Eid::from(eid), prop_name).await;
        }
    }
}
```

---

## Special Cases

### 1. Parameterized Property Names

```cypher
-- Property name is a runtime parameter
WHERE validAt(e, $startProp, $endProp, $timestamp)
```

**Problem:** Can't determine property names at plan time.

**Solution:** Two options:

```rust
// Option A: Require * (all properties) when property name is parameter
if args.iter().any(|a| a.is_parameter()) {
    requirements.insert("*".to_string());
}

// Option B: Defer to eager hydration for this specific call
// Mark the function call for runtime hydration
```

### 2. Conditional Property Access

```cypher
-- Property might not be accessed
RETURN CASE WHEN x THEN e.role ELSE 'N/A' END
```

**Solution:** Analyze both branches, union requirements:

```rust
Expr::Case { when_clauses, else_clause } => {
    for (_, result) in when_clauses {
        self.analyze(result);  // Might access e.role
    }
    if let Some(else_expr) = else_clause {
        self.analyze(else_expr);
    }
}
```

### 3. Dynamic Property Access

```cypher
-- Property name computed at runtime
RETURN e[propName]
```

**Solution:** Require `*` for dynamic access:

```rust
Expr::DynamicProperty(var, _) => {
    let var_name = var.as_variable_name();
    self.requirements.entry(var_name).or_default().insert("*".to_string());
}
```

### 4. Subqueries and CALL

```cypher
MATCH (p)-[e]->(c)
CALL {
    WITH e
    RETURN e.role AS r
}
```

**Solution:** Analyze subqueries, propagate requirements to outer scope:

```rust
LogicalPlan::Call { subquery, imports } => {
    // Analyze subquery
    let sub_reqs = analyze_requirements(subquery);
    
    // Propagate requirements for imported variables
    for var in imports {
        if let Some(reqs) = sub_reqs.get(var) {
            self.requirements.entry(var.clone()).or_default().extend(reqs.clone());
        }
    }
}
```

### 5. RETURN e (Full Entity)

```cypher
-- Returning the whole entity
RETURN e
```

**Solution:** Require all properties:

```rust
Expr::Variable(name) if in_return_clause => {
    self.requirements.entry(name.clone()).or_default().insert("*".to_string());
}
```

---

## Query Rewriting vs Pushdown

Both are valid optimization strategies. They can coexist:

### Strategy Comparison

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    OPTIMIZATION STRATEGY MATRIX                         │
└─────────────────────────────────────────────────────────────────────────┘

                        Can Eliminate Function?
                        ────────────────────────
                              YES         NO
                           ┌─────────┬─────────┐
         Function is       │ QUERY   │ PUSHDOWN│
         Known Temporal    │ REWRITE │ HYDRATE │
                           │ ⭐⭐⭐⭐⭐ │ ⭐⭐⭐⭐  │
                           ├─────────┼─────────┤
         Function is       │   N/A   │ PUSHDOWN│
         Unknown/UDF       │         │ HYDRATE │
                           │         │ ⭐⭐⭐⭐  │
                           └─────────┴─────────┘
```

### Optimization Pipeline

```rust
fn optimize_query(plan: LogicalPlan) -> LogicalPlan {
    let plan = plan
        // First: try to eliminate functions entirely
        .apply(RewriteTemporalFunctions)
        
        // Then: pushdown remaining property requirements
        .apply(PushdownPropertyRequirements)
        
        // Standard optimizations
        .apply(PredicatePushdown)
        .apply(ProjectionPushdown)
        .apply(JoinReordering);
    
    plan
}
```

### Example: Combined Optimization

```cypher
-- Original
MATCH (p)-[e:WORKS]->(c)
WHERE validAt(e, 'start', 'end', $ts)
  AND customUdf(e)
RETURN e.role

-- After RewriteTemporalFunctions
MATCH (p)-[e:WORKS]->(c)
WHERE e.start <= $ts AND e.end >= $ts  -- Rewritten!
  AND customUdf(e)                      -- Can't rewrite, unknown UDF
RETURN e.role

-- After PushdownPropertyRequirements
-- e requires: {start, end, role, *}  -- * because customUdf is unknown
-- With function registry for customUdf, could be: {start, end, role, udf_prop}
```

---

## Implementation Roadmap

### Phase 1: Analysis Infrastructure (Week 1-2)

```
□ Implement PropertyAnalyzer AST walker
□ Add FunctionPropertySpec registry
□ Add required_properties field to scan plan nodes
□ Unit tests for requirement analysis
```

### Phase 2: Propagation (Week 2-3)

```
□ Implement RequirementPropagator 
□ Handle all plan node types
□ Handle subqueries and CALL
□ Integration tests for propagation
```

### Phase 3: Scan Modification (Week 3-4)

```
□ Modify EdgeScan to accept property requirements
□ Implement batch property fetching
□ Optimize to single joined scan where possible
□ Performance benchmarks
```

### Phase 4: Execution Integration (Week 4-5)

```
□ Update row format to carry hydrated entities
□ Add fallback for missed properties
□ Remove eager hydration workaround (or keep as fallback)
□ End-to-end tests
```

### Phase 5: Query Rewriting (Week 5-6)

```
□ Implement temporal function rewrite rules
□ Add rewrite pass to optimizer pipeline
□ Combine with pushdown for remaining functions
□ Performance comparison
```

---

## Performance Analysis

### Theoretical Complexity

| Approach | Storage Scans | Time Complexity |
|----------|---------------|-----------------|
| Lazy (current) | N × M | O(N × M) |
| Eager hydration | 2N | O(N) |
| Pushdown hydration | N | O(N) |
| Query rewrite + pushdown | N | O(N) with better constants |

Where N = number of edges, M = properties accessed per edge.

### Expected Improvements

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Benchmark: 10,000 edges, 5 properties each, 3 accessed in query         │
└─────────────────────────────────────────────────────────────────────────┘

Lazy Loading (current):
  - Adjacency scan: 1
  - Property scans: 10,000 × 3 = 30,000
  - Total scans: 30,001
  - Estimated time: 30s

Eager Hydration:
  - Adjacency scan: 1  
  - Hydration scans: 10,000 (all props per edge)
  - Total scans: 10,001
  - Estimated time: 10s

Pushdown Hydration:
  - Combined scan: 1 (adjacency + 3 projected columns)
  - Total scans: 1
  - Estimated time: 0.1s

Query Rewrite + DataFusion:
  - Filter pushed to Lance: 1 scan with predicate
  - Total scans: 1
  - Estimated time: 0.05s (filter evaluated in storage)
```

### Memory Impact

```
Without hydration:
  Row size: ~100 bytes (IDs only)
  10K rows: 1 MB

With full hydration:
  Row size: ~500 bytes (IDs + properties)
  10K rows: 5 MB

With selective hydration (pushdown):
  Row size: ~200 bytes (IDs + 3 properties)
  10K rows: 2 MB
```

---

## Summary

Pushdown hydration is the architecturally correct solution because:

1. **Single storage access** — properties loaded with topology
2. **Selective loading** — only fetch what's needed
3. **Works with existing execution** — minimal changes to expression evaluation
4. **Composable with query rewriting** — eliminate functions when possible, pushdown for the rest
5. **Future-proof** — same pattern works for vertices, paths, subqueries

### Decision Matrix

| Scenario | Recommended Approach |
|----------|---------------------|
| Temporal functions (validAt, overlaps) | Query rewrite |
| Known UDFs with property specs | Pushdown hydration |
| Unknown UDFs | Pushdown with `*` |
| Dynamic property access | Pushdown with `*` |
| RETURN entity | Pushdown with `*` |
| Simple property filters | Standard predicate pushdown |

---

## Appendix: Full Example

### Query

```cypher
MATCH (p:Person)-[e:EMPLOYED_BY]->(c:Company)
WHERE uni.temporal.validAt(e, 'valid_from', 'valid_to', datetime('2021-06-15'))
  AND p.age > 30
RETURN p.name, e.role, c.name
```

### Phase 1 Output

```rust
PropertyRequirements {
    "p": {"age", "name"},
    "e": {"valid_from", "valid_to", "role"},
    "c": {"name"},
}
```

### Phase 2 Output (Annotated Plan)

```
Project [p.name, e.role, c.name]
  Filter [validAt(e, ...) AND p.age > 30]
    Expand [e:EMPLOYED_BY, required: {valid_from, valid_to, role}]
      NodeScan [p:Person, required: {age, name}]
      NodeScan [c:Company, required: {name}]
```

### Phase 3: Generated Scan

```sql
-- For edges (pseudo-SQL)
SELECT _eid, _src, _dst, _type, valid_from, valid_to, role
FROM edge_dataset
WHERE _type = 'EMPLOYED_BY'

-- For Person nodes
SELECT _vid, _label, age, name
FROM vertex_dataset  
WHERE _label = 'Person'

-- For Company nodes
SELECT _vid, _label, name
FROM vertex_dataset
WHERE _label = 'Company'
```

### Phase 4: Execution

Rows contain fully hydrated entities, functions receive complete property maps, no lazy loading triggered.

---

**End of Document**
