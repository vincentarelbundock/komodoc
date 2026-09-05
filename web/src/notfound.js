import { mount } from "svelte";
import "./styles/app.css";
import NotFound from "./components/NotFound.svelte";

mount(NotFound, { target: document.getElementById("app") });
