import { mount } from "svelte";
import "./styles/app.css";
import Landing from "./components/Landing.svelte";

mount(Landing, { target: document.getElementById("app") });
