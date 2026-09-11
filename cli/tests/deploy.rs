// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **The deployment samples name what the workflow actually publishes**, checked the way
//! `packaging.rs` checks that every binary is packaged: read the workflow and the samples, and
//! fail the build when they disagree.
//!
//! `ui_web/deploy/` holds one sample per thing people actually run — `compose.yaml` and
//! `kubernetes.yaml` — plus the two ways to reach that Service from outside a cluster,
//! `httproute.yaml` (Gateway API) and `ingress.yaml` (Ingress API), of which a given cluster
//! wants one. All of them are copy-and-run instructions rather than prose, which is exactly what
//! makes them rot silently: the image tag lives in `.github/workflows/container.yml`, the port
//! and the health endpoint are settings of the running server, and nothing about a stale sample
//! fails until somebody pastes it into a terminal. A sample nobody runs is `doc/plan.md` rule 4's
//! UI-only feature in another costume.
//!
//! What is checked here is only the agreement *between files* — the things one edit can break
//! in another file, plus one fact about the world that a comment would not hold anybody to:
//! `ingress-nginx` is retired, so neither routing sample may *apply* its annotations, however
//! much both of them discuss it.
//!
//! That the samples run at all is not a unit test's question: it needs a daemon and a cluster,
//! and it was answered by running them — Compose end to end, the pod spec under
//! `podman kube play`, and every manifest through `kubeconform -strict`, the Gateway API ones
//! against the CRD catalog's schemas.
//!
//! Everything is `include_str!`, read at compile time, so this cannot pass by looking in the
//! wrong place at runtime.

const WORKFLOW: &str = include_str!("../../.github/workflows/container.yml");
const COMPOSE: &str = include_str!("../../ui_web/deploy/compose.yaml");
const KUBERNETES: &str = include_str!("../../ui_web/deploy/kubernetes.yaml");
const HTTPROUTE: &str = include_str!("../../ui_web/deploy/httproute.yaml");
const INGRESS: &str = include_str!("../../ui_web/deploy/ingress.yaml");

/// A sample with the comments taken out — what the cluster is actually told, as opposed to what
/// the file explains. Both routing samples *discuss* `ingress-nginx` and its annotations at
/// length, so a check that nothing points at that controller has to be able to tell a line of
/// prose from a line of configuration.
fn live_lines(sample: &str) -> String {
    sample
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The image `container.yml`'s `web-image` job builds and pushes, read out of the workflow
/// rather than spelled a second time here.
fn published_image() -> &'static str {
    let line = WORKFLOW
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("IMAGE:") && l.contains("grind-web"))
        .expect("container.yml declares an IMAGE for the web shell");
    line.trim_start_matches("IMAGE:").trim()
}

#[test]
fn both_samples_deploy_the_image_the_workflow_publishes() {
    let image = published_image();
    assert_eq!(
        image, "ghcr.io/fwilhe2/grind-web",
        "the published web image moved; the samples in ui_web/deploy/ name the old one"
    );

    for (name, sample) in [("compose.yaml", COMPOSE), ("kubernetes.yaml", KUBERNETES)] {
        assert!(
            sample.contains(&format!("{image}:latest")),
            "ui_web/deploy/{name} does not deploy {image}:latest, which is what \
             container.yml publishes"
        );
    }
}

#[test]
fn the_source_build_is_the_dockerfile_the_workflow_builds() {
    // The compose file's `source` profile and the workflow's `web-image` job must build the
    // same thing the same way: this Dockerfile, with the repository root as the context,
    // because grind-web depends on grind-core, grind-sheet and grind-text.
    assert!(
        WORKFLOW.contains("file: ./ui_web/Dockerfile"),
        "container.yml no longer builds ui_web/Dockerfile"
    );
    assert!(
        COMPOSE.contains("dockerfile: ui_web/Dockerfile"),
        "compose.yaml's source profile builds a different Dockerfile than the workflow does"
    );
    assert!(
        COMPOSE.contains("context: ../.."),
        "compose.yaml's build context must be the repository root, not ui_web/"
    );
}

#[test]
fn the_port_served_is_the_port_published_and_probed() {
    // The image's own default is 80, and both samples override it — a container that runs as a
    // non-root user cannot bind a privileged port. Every other number in the sample has to
    // follow that one, and each of these is a different file's idea of it.
    assert!(
        COMPOSE.contains(r#"SERVER_PORT: "8080""#),
        "compose.yaml no longer sets SERVER_PORT"
    );
    assert!(
        COMPOSE.contains(r#"- "8080:8080""#),
        "compose.yaml publishes a port the server is not listening on"
    );

    assert!(
        KUBERNETES.contains("containerPort: 8080"),
        "kubernetes.yaml's containerPort disagrees with SERVER_PORT"
    );
    assert!(
        KUBERNETES.contains("name: SERVER_PORT") && KUBERNETES.contains(r#"value: "8080""#),
        "kubernetes.yaml no longer sets SERVER_PORT to the port it declares"
    );
}

#[test]
fn the_probes_have_the_endpoint_that_answers_them() {
    // /health is a 404 unless SERVER_HEALTH is set, so a probe without it fails every pod.
    // Measured against the published image, not assumed.
    assert!(
        KUBERNETES.contains("name: SERVER_HEALTH"),
        "kubernetes.yaml probes /health without enabling it; every pod would fail readiness"
    );
    assert_eq!(
        KUBERNETES.matches("path: /health").count(),
        2,
        "kubernetes.yaml should probe /health from both the readiness and the liveness probe"
    );

    // There is no shell in a distroless image, so an `exec` probe cannot work here — the
    // kubelet's own httpGet is the only kind that needs nothing inside the container.
    assert!(
        !KUBERNETES.contains("exec:"),
        "an exec probe cannot run in a distroless image; use httpGet"
    );
}

#[test]
fn the_pod_is_unprivileged_and_read_only() {
    // The namespace enforces the `restricted` Pod Security Standard, so a pod spec that loses
    // one of these stops being admitted rather than quietly running with more than it needs.
    // The container was verified to need nothing writable and no capability at all.
    for required in [
        "pod-security.kubernetes.io/enforce: restricted",
        "runAsNonRoot: true",
        "readOnlyRootFilesystem: true",
        "allowPrivilegeEscalation: false",
        "type: RuntimeDefault",
        r#"drop: ["ALL"]"#,
    ] {
        assert!(
            KUBERNETES.contains(required),
            "kubernetes.yaml no longer says `{required}`, which the restricted \
             Pod Security Standard requires of it"
        );
    }

    for required in ["read_only: true", r#"user: "65532:65532""#, "- ALL"] {
        assert!(
            COMPOSE.contains(required),
            "compose.yaml no longer says `{required}`"
        );
    }
}

#[test]
fn both_routing_samples_point_at_the_service_that_exists() {
    // Three files, one Service. The Ingress asks for it by the port's *name*, the HTTPRoute by
    // its number — that is the two APIs' own spelling, not a choice made here — so the Service
    // has to keep both.
    assert!(
        KUBERNETES.contains("kind: Service") && KUBERNETES.contains("targetPort: http"),
        "kubernetes.yaml's Service lost the named port the Ingress refers to"
    );
    assert!(
        KUBERNETES.contains("port: 80"),
        "kubernetes.yaml's Service lost port 80, which httproute.yaml's backendRef names"
    );

    for (name, sample) in [("httproute.yaml", HTTPROUTE), ("ingress.yaml", INGRESS)] {
        assert!(
            sample.contains("name: grind-web"),
            "ui_web/deploy/{name} does not route to the grind-web Service"
        );
        assert!(
            sample.contains("namespace: grind"),
            "ui_web/deploy/{name} disagrees with kubernetes.yaml about the namespace"
        );
    }

    assert!(
        HTTPROUTE.contains("backendRefs:") && HTTPROUTE.contains("port: 80"),
        "httproute.yaml's backendRef no longer names the Service's port"
    );
    assert!(
        INGRESS.contains("name: http"),
        "ingress.yaml no longer names the Service's http port"
    );
}

#[test]
fn the_gateway_api_sample_is_on_the_ga_group_version() {
    // GatewayClass, Gateway and HTTPRoute are GA. A sample that drifted back to an alpha or beta
    // group would be telling people to write something their cluster may no longer serve.
    assert!(
        HTTPROUTE.contains("apiVersion: gateway.networking.k8s.io/v1"),
        "httproute.yaml is not on the GA Gateway API group version"
    );
    for prerelease in [
        "gateway.networking.k8s.io/v1alpha2",
        "gateway.networking.k8s.io/v1beta1",
    ] {
        assert!(
            !live_lines(HTTPROUTE).contains(prerelease),
            "httproute.yaml uses the pre-GA group version {prerelease}"
        );
    }

    // The one redirect this file needs is HTTP to HTTPS, which is Core conformance — a scheme
    // redirect with a 301. Anything Extended (the subpath rewrite, a path redirect) stays
    // commented, since it needs the controller's conformance report checked first.
    assert!(
        HTTPROUTE.contains("type: RequestRedirect") && HTTPROUTE.contains("scheme: https"),
        "httproute.yaml lost the HTTP-to-HTTPS redirect"
    );
    assert!(
        !live_lines(HTTPROUTE).contains("URLRewrite"),
        "httproute.yaml promotes an Extended-conformance filter into the applied manifest; \
         the subpath variant is commented on purpose"
    );
}

#[test]
fn neither_routing_sample_points_at_the_retired_controller() {
    // `kubernetes/ingress-nginx` reached end of life on 31 March 2026: archived, no releases and
    // no security fixes, in something that sits in the HTTP data path. Both files *discuss* it —
    // that is the whole explanation for why there are two of them — so this reads what the
    // cluster is told rather than what the comments say.
    for (name, sample) in [("httproute.yaml", HTTPROUTE), ("ingress.yaml", INGRESS)] {
        let live = live_lines(sample);
        assert!(
            !live.contains("nginx.ingress.kubernetes.io/"),
            "ui_web/deploy/{name} applies an ingress-nginx annotation; that controller is retired"
        );
        assert!(
            !live.contains("ingressClassName: nginx"),
            "ui_web/deploy/{name} defaults to the retired ingress-nginx controller"
        );
    }
}
